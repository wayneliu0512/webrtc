use anyhow::{anyhow, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use std::os::fd::AsRawFd;
use std::sync::{Arc, Weak};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{error, info};
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
use webrtc::rtp::packet::Packet;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::rtp_transceiver::rtp_receiver::RTCRtpReceiver;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_local::{TrackLocal, TrackLocalWriter};
use webrtc::track::track_remote::TrackRemote;
use webrtc::util::Unmarshal;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

use crate::portal;

/// Creates a new WebRTC PeerConnection with screencasting capabilities.
pub async fn create_peer_connection(
    tx: UnboundedSender<String>,
) -> Result<Arc<RTCPeerConnection>> {
    // 1. Basic WebRTC API and Configuration setup
    let api = create_api()?;
    let config = RTCConfiguration {
        ice_servers: vec![RTCIceServer {
            // urls: vec!["stun:stun.l.google.com:19302".to_owned()],
            ..Default::default()
        }],
        ..Default::default()
    };

    // 2. Create Peer Connection
    let peer_connection = Arc::new(api.new_peer_connection(config).await?);

    // 3. Create and Add Local Video Track
    let local_track = create_local_track();
    let rtp_sender = peer_connection
        .add_track(Arc::clone(&local_track) as Arc<dyn TrackLocal + Send + Sync>)
        .await?;

    // Read incoming RTCP packets
    // Before these packets are returned they are processed by interceptors. For things
    // like NACK this needs to be called.
    let (pli_tx, pli_rx) = unbounded_channel();
    tokio::spawn(async move {
        let mut rtcp_buf = vec![0u8; 1500];
        while let Ok((_, _)) = rtp_sender.read(&mut rtcp_buf).await {
            // For now, we just blindly forward any RTCP packet as a PLI signal
            // In a real app you might parse it to ensure strictly PLI or FIR.
            // But usually the browser only sends RTCP for these reasons on a video sender.
            let _ = pli_tx.send(());
        }
    });

    // 4. Register Handlers
    // Handle incoming tracks (Placeholder for now)
    peer_connection.on_track(Box::new(
        move |track: Arc<TrackRemote>, _receiver: Arc<RTCRtpReceiver>, _transceiver| {
            Box::pin(async move {
                info!("Track {} received", track.id());
            })
        },
    ));

    setup_data_channel_handling(&peer_connection);
    setup_ice_handling(&peer_connection, tx);
    setup_connection_state_handling(&peer_connection);

    // 5. Spawn Screencast Task
    // We pass a Weak reference to the PC so the screencast loop can stop if PC is dropped/closed.
    let pc_weak = Arc::downgrade(&peer_connection);
    tokio::spawn(async move {
        if let Err(e) = run_screencast_loop(local_track, pc_weak, pli_rx).await {
            error!("Screencast task encountered error: {}", e);
        }
    });

    Ok(peer_connection)
}

/// Helper to create the WebRTC API with default media engine and interceptors.
fn create_api() -> Result<webrtc::api::API> {
    let mut m = MediaEngine::default();
    m.register_default_codecs()?;
    
    let mut registry = Registry::new();
    registry = register_default_interceptors(registry, &mut m)?;
    
    Ok(APIBuilder::new()
        .with_media_engine(m)
        .with_interceptor_registry(registry)
        .build())
}

/// Helper to create the local video track for screencasting.
fn create_local_track() -> Arc<TrackLocalStaticRTP> {
    Arc::new(TrackLocalStaticRTP::new(
        RTCRtpCodecCapability {
            mime_type: MIME_TYPE_VP8.to_owned(),
            ..Default::default()
        },
        "video".to_owned(),
        "webrtc-rs".to_owned(),
    ))
}

/// Sets up Data Channel handlers (e.g. for Echo).
fn setup_data_channel_handling(pc: &Arc<RTCPeerConnection>) {
    pc.on_data_channel(Box::new(move |d: Arc<RTCDataChannel>| {
        let d_label = d.label().to_owned();
        let d_id = d.id();
        info!("New DataChannel {} {}", d_label, d_id);

        Box::pin(async move {
            let d_clone = d.clone();
            let d_label_clone = d_label.clone();
            
            d.on_open(Box::new(move || {
                info!("Data channel '{}'-'{}' open", d_label_clone, d_id);
                
                Box::pin(async move {
                    let d_label_inner = d_label_clone.clone();
                    let d_inner = d_clone.clone();
                    
                    d_clone.on_message(Box::new(move |msg: DataChannelMessage| {
                        let msg_str = std::str::from_utf8(&msg.data).unwrap_or("<invalid utf8>");
                        info!("Message from DataChannel '{}': '{}'", d_label_inner, msg_str);

                        let d_response = d_inner.clone();
                        let reply = format!("Echo from Rust: {}", msg_str);
                        Box::pin(async move {
                            if let Err(e) = d_response.send_text(reply).await {
                                error!("Failed to send data channel reply: {}", e);
                            }
                        })
                    }));
                })
            }));
        })
    }));
}

/// Sets up ICE Candidate handling (sending local candidates to signaling server).
fn setup_ice_handling(pc: &Arc<RTCPeerConnection>, tx: UnboundedSender<String>) {
    pc.on_ice_candidate(Box::new(move |c: Option<RTCIceCandidate>| {
        let tx = tx.clone();
        Box::pin(async move {
            if let Some(candidate) = c {
                if let Ok(candidate_json) = candidate.to_json() {
                    info!("Generated Local Candidate: {:?}", candidate_json);
                    
                    let json_candidate = serde_json::json!({
                        "type": "candidate",
                        "candidate": candidate_json
                    });
                     
                    let _ = tx.send(json_candidate.to_string());
                }
            }
        })
    }));
}

/// Sets up Connection State monitoring and logging.
fn setup_connection_state_handling(pc: &Arc<RTCPeerConnection>) {
    let pc_weak = Arc::downgrade(pc);
    
    // Monitor ICE Connection State
    pc.on_ice_connection_state_change(Box::new(move |connection_state: webrtc::ice_transport::ice_connection_state::RTCIceConnectionState| {
        let pc_weak = pc_weak.clone();
        Box::pin(async move {
            info!("Connection State has changed {}", connection_state);
            if connection_state == webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Connected {
                if let Some(pc) = pc_weak.upgrade() {
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

    // Monitor Overall Peer Connection State
    pc.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
        Box::pin(async move {
            info!("Peer Connection State has changed: {}", s);
            if s == RTCPeerConnectionState::Failed {
                info!("Peer Connection has gone to failed exiting");
            }
        })
    }));
}

/// The main loop for the Screencast task:
/// 1. Initializes GStreamer
/// 2. Requests Portal Access
/// 3. Builds and runs the Pipeline
/// 4. Pushes RTP packets to the Track
async fn run_screencast_loop(
    local_track: Arc<TrackLocalStaticRTP>,
    pc_weak: Weak<RTCPeerConnection>,
    mut pli_rx: UnboundedReceiver<()>,
) -> Result<()> {
    info!("Starting screencast setup...");
    
    // Initialize GStreamer
    gst::init().map_err(|e| anyhow!("Failed to init GStreamer: {}", e))?;

    // Open Portal
    info!("Requesting portal access...");
    let (session, stream, fd) = portal::open_portal().await
        .map_err(|e| anyhow!("Failed to open portal: {}", e))?;

    let node_id = stream.pipe_wire_node_id();
    let fd_raw = fd.as_raw_fd();
    info!("Got portal stream: node_id={}, fd={}", node_id, fd_raw);

    // Construct pipeline
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
    enc.set_property("keyframe-max-dist", 60i32); // Send a keyframe every ~2s
    enc.set_property("deadline", 1i64);

    // Listen for PLI from WebRTC and force keyframe
    let enc_clone = enc.clone();
    tokio::spawn(async move {
        while let Some(_) = pli_rx.recv().await {
            tracing::trace!("Received PLI, forcing keyframe");
            let mut structure = gst::Structure::new_empty("GstForceKeyUnit");
            structure.set("all-headers", true);
            let event = gst::event::CustomUpstream::new(structure);
            enc_clone.send_event(event);
        }
    });

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
        if pc_weak.upgrade().is_none() {
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
                        if let Err(e) = local_track.write_rtp(&packet).await {
                            error!("Failed to write RTP: {}", e);
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
    
    // Cleanup
    let _ = pipeline.set_state(gst::State::Null);
    let _ = session.close().await;
    
    Ok(())
}
