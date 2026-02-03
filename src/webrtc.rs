use anyhow::Result;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use tracing::info;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MIME_TYPE_VP8, MediaEngine};
use webrtc::api::APIBuilder;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_candidate::RTCIceCandidate;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::rtp_transceiver::rtp_receiver::RTCRtpReceiver;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_local::{TrackLocal, TrackLocalWriter};
use webrtc::track::track_remote::TrackRemote;
use crate::portal;
use std::os::fd::AsRawFd;
use gstreamer::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;
use anyhow::anyhow;
use webrtc::rtp::packet::Packet;
use webrtc::util::Unmarshal;

pub async fn create_peer_connection(
    tx: UnboundedSender<String>,
) -> Result<Arc<RTCPeerConnection>> {
    // Create a MediaEngine object to configure the supported codec
    let mut m = MediaEngine::default();

    // Register default codecs
    m.register_default_codecs()?;

    // Create a InterceptorRegistry. This is the user configurable RTP/RTCP Pipeline.
    // This provides NACKs, RTCP Reports and other features. If you use `webrtc::api::API` the
    // default InterceptorRegistry is used. If you want to register your own interceptors you
    // can do so here.
    let mut registry = Registry::new();

    // Use the default set of Interceptors
    registry = register_default_interceptors(registry, &mut m)?;

    // Create the API object with the MediaEngine
    let api = APIBuilder::new()
        .with_media_engine(m)
        .with_interceptor_registry(registry)
        .build();

    // Prepare the configuration
    let config = RTCConfiguration {
        ice_servers: vec![RTCIceServer {
            // urls: vec!["stun:stun.l.google.com:19302".to_owned()],
            ..Default::default()
        }],
        ..Default::default()
    };

    // Create a new RTCPeerConnection
    let peer_connection = Arc::new(api.new_peer_connection(config).await?);

    // Create a video track
    let local_track = Arc::new(TrackLocalStaticRTP::new(
        RTCRtpCodecCapability {
            mime_type: MIME_TYPE_VP8.to_owned(),
            ..Default::default()
        },
        "video".to_owned(),
        "webrtc-rs".to_owned(),
    ));

    // Add this track to be sent
    let _ = peer_connection
        .add_track(Arc::clone(&local_track) as Arc<dyn TrackLocal + Send + Sync>)
        .await?;

    // Register on_track handler - we don't expect remote tracks for screencast, 
    // but keep it for debugging or future requirements if needed.
    peer_connection.on_track(Box::new(
        move |track: Arc<TrackRemote>, _receiver: Arc<RTCRtpReceiver>, _transceiver| {
            Box::pin(async move {
                info!("Track {} received", track.id());
            })
        },
    ));

    // Spawn the screencast source task
    let local_track_clone = local_track.clone();
    let peer_connection_clone = Arc::downgrade(&peer_connection);
    
    tokio::spawn(async move {
        let task = async move {
            info!("Starting screencast setup...");
            
            // Initialize GStreamer
            gst::init().map_err(|e| anyhow!("Failed to init GStreamer: {}", e))?;

            // Open Portal
            info!("Requesting portal access...");
            let (session, stream, fd) = portal::open_portal().await
                .map_err(|e| anyhow!("Failed to open portal: {}", e))?;

            // pipe_wire_node_id might be the correct field/method.
            // pipe_wire_node_id is a field in ashpd 0.9 Stream struct
            let node_id = stream.pipe_wire_node_id();
            let fd_raw = fd.as_raw_fd();
            info!("Got portal stream: node_id={}, fd={}", node_id, fd_raw);

            // Construct pipeline
            // pipewiresrc needs fd to be valid during the pipeline lifecycle. 
            // We pass fd index, but we must keep `fd` (OrderedFd) alive.
            // Also ensure high-quality, low-latency encoding.
            let pipeline = gst::Pipeline::new();

            let src = gst::ElementFactory::make("pipewiresrc")
                .build()
                .map_err(|e| anyhow!("Failed to create src: {}", e))?;
            src.set_property("fd", fd_raw);
            src.set_property("path", &node_id.to_string());
            src.set_property("always-copy", true);

            let conv = gst::ElementFactory::make("videoconvert")
                .build()
                .map_err(|e| anyhow!("Failed to create conv: {}", e))?;

            let enc = gst::ElementFactory::make("vp8enc")
                 .build()
                 .map_err(|e| anyhow!("Failed to create enc: {}", e))?;
            enc.set_property_from_str("error-resilient", "partitions");
            enc.set_property("key-frame-max-dist", 2000i32);
            enc.set_property("deadline", 1i64);

            let pay = gst::ElementFactory::make("rtpvp8pay")
                 .build()
                 .map_err(|e| anyhow!("Failed to create pay: {}", e))?;

            let sink = gst::ElementFactory::make("appsink")
                 .build()
                 .map_err(|e| anyhow!("Failed to create sink: {}", e))?;

            pipeline.add_many(&[&src, &conv, &enc, &pay, &sink])
                .map_err(|e| anyhow!("Failed to add elements: {}", e))?;

            gst::Element::link_many(&[&src, &conv, &enc, &pay, &sink])
                .map_err(|e| anyhow!("Failed to link elements: {}", e))?;

            let appsink = sink.downcast::<gst_app::AppSink>()
                .map_err(|_| anyhow!("Failed to cast appsink"))?;

            // Start pipeline
            pipeline.set_state(gst::State::Playing)
                .map_err(|e| anyhow!("Failed to set pipeline state to Playing: {}", e))?;
            
            info!("Pipeline playing");

            // Loop to pull samples
            loop {
                // Check if peer connection is still alive
                 if peer_connection_clone.upgrade().is_none() {
                    break;
                }

                match appsink.pull_sample() {
                    Ok(sample) => {
                        let buffer: &gst::BufferRef = match sample.buffer() {
                            Some(b) => b,
                            None => continue,
                        };

                        if let Ok(map) = buffer.map_readable() {
                            let mut buf: &[u8] = &map;
                            // Parse RTP packet
                            if let Ok(packet) = Packet::unmarshal(&mut buf) {
                                if let Err(e) = local_track_clone.write_rtp(&packet).await {
                                    tracing::error!("Failed to write RTP: {}", e);
                                    if e.to_string().contains("closed") {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    Err(_e) => {
                        // EOS or error
                        break;
                    }
                }
            }
            
            // Screencast task ending, cleaning up...
            let _ = pipeline.set_state(gst::State::Null);
            let _ = session.close().await;
            // fd drops here, which is fine as pipeline is stopped
            Ok::<(), anyhow::Error>(())
        };

        if let Err(e) = task.await {
            info!("Screencast task encountered error: {}", e);
        }
    });

    // Register data channel creation handling
    peer_connection.on_data_channel(Box::new(move |d: Arc<RTCDataChannel>| {
        let d_label = d.label().to_owned();
        let d_id = d.id();
        info!("New DataChannel {} {}", d_label, d_id);

        // Register channel opening handling
        Box::pin(async move {
            let d2 = d.clone();
            let d_label2 = d_label.clone();
            let d_id2 = d_id;
            d.on_open(Box::new(move || {
                info!("Data channel '{}'-'{}' open", d_label2, d_id2);

                Box::pin(async move {
                    let d_label3 = d_label2.clone();
                    let d3 = d2.clone();
                    d2.on_message(Box::new(move |msg: DataChannelMessage| {
                        let msg_str = String::from_utf8(msg.data.to_vec()).unwrap();
                        info!("Message from DataChannel '{}': '{}'", d_label3, msg_str);

                        let d4 = d3.clone();
                        Box::pin(async move {
                            let reply = format!("Echo from Rust: {}", msg_str);
                            let _ = d4.send_text(reply).await;
                        })
                    }));
                })
            }));
        })
    }));

    // Register on_ice_candidate handler
    let tx_clone = tx.clone();
    peer_connection.on_ice_candidate(Box::new(move |c: Option<RTCIceCandidate>| {
        let tx = tx_clone.clone();
        Box::pin(async move {
            if let Some(candidate) = c {
                if let Ok(candidate_json) = candidate.to_json() {
                    info!("Generated Local Candidate: {:?}", candidate_json);
                    
                    let json_candidate = serde_json::json!({
                        "type": "candidate",
                        "candidate": candidate_json
                    });
                     // ignore error if channel closed
                    let _ = tx.send(json_candidate.to_string());
                }
            }
        })
    }));

    // Register on_ice_connection_state_change
    let pc_state = Arc::downgrade(&peer_connection);
    peer_connection.on_ice_connection_state_change(Box::new(move |connection_state: webrtc::ice_transport::ice_connection_state::RTCIceConnectionState| {
        let pc_state = pc_state.clone();
        Box::pin(async move {
            info!("Connection State has changed {}", connection_state);
            if connection_state == webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Connected {
                if let Some(pc) = pc_state.upgrade() {
                    // Access ICE transport via SCTP (DataChannel)
                    let sctp = pc.sctp();
                    let dtls_transport = sctp.transport();
                    let ice_transport = dtls_transport.ice_transport();
                    
                    if let Some(pair) = ice_transport.get_selected_candidate_pair().await {
                        info!("Selected Candidate Pair: {:?}", pair);
                    }
                }
            }
        })
    }));
    
    // Allow verifying peer connection state
    peer_connection.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
        Box::pin(async move {
            info!("Peer Connection State has changed: {}", s);

            if s == RTCPeerConnectionState::Failed {
                info!("Peer Connection has gone to failed exiting");
            }
        })
    }));

    Ok(peer_connection)
}
