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
use webrtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;


use crate::portal::{self, PortalInput};
use edge_lib::protocol::webrtc::input_event::InputEvent;

/// Creates a new WebRTC PeerConnection with remote desktop capabilities.
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

    let (pli_tx, pli_rx) = unbounded_channel::<()>();

    // Spawn RTCP Reader for PLI
    let rtp_sender_clone = rtp_sender.clone();
    tokio::spawn(async move {
        let mut rtcp_buf = vec![0u8; 1500];
        while let Ok((packets, _)) = rtp_sender_clone.read(&mut rtcp_buf).await {
            for packet in packets {
                if packet.as_any().downcast_ref::<PictureLossIndication>().is_some() {
                    let _ = pli_tx.send(());
                }
            }
        }
    });

    let (input_tx, input_rx) = unbounded_channel::<InputEvent>();

    // 4. Register Handlers
    // Handle incoming tracks (Placeholder for now)
    peer_connection.on_track(Box::new(
        move |track: Arc<TrackRemote>, _receiver: Arc<RTCRtpReceiver>, _transceiver| {
            Box::pin(async move {
                info!("Track {} received", track.id());
            })
        },
    ));

    setup_data_channel_handling(&peer_connection, input_tx);
    setup_ice_handling(&peer_connection, tx);
    setup_connection_state_handling(&peer_connection);

    // 5. Spawn Remote Desktop Task
    // We pass a Weak reference to the PC so the remote desktop loop can stop if PC is dropped/closed.
    let pc_weak = Arc::downgrade(&peer_connection);
    tokio::spawn(async move {
        if let Err(e) = run_remote_desktop_loop(local_track, pc_weak, pli_rx, input_rx).await {
            error!("Remote desktop task encountered error: {}", e);
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

/// Sets up Data Channel handlers.
fn setup_data_channel_handling(
    pc: &Arc<RTCPeerConnection>,
    input_tx: UnboundedSender<InputEvent>,
) {
    pc.on_data_channel(Box::new(move |d: Arc<RTCDataChannel>| {
        let d_label = d.label().to_owned();
        let d_id = d.id();
        info!("New DataChannel {} {}", d_label, d_id);

        let input_tx = input_tx.clone();

        Box::pin(async move {
            let d_label_clone = d_label.clone();
            let d_clone = d.clone();
            
            d.on_open(Box::new(move || {
                info!("Data channel '{}'-'{}' open", d_label_clone, d_id);
                
                let d_clone = d_clone.clone();
                let d_label_clone = d_label_clone.clone();
                let input_tx = input_tx.clone();

                Box::pin(async move {
                    let d_label_inner = d_label_clone.clone();
                    
                    d_clone.on_message(Box::new(move |msg: DataChannelMessage| {
                        let input_tx = input_tx.clone();
                        let d_label = d_label_inner.clone();

                        Box::pin(async move {
                            match serde_json::from_slice::<InputEvent>(&msg.data) {
                                Ok(event) => {
                                    if let Err(e) = input_tx.send(event) {
                                        error!("Failed to forward input event: {}", e);
                                    }
                                }
                                Err(e) => {
                                    // Log only if it's not strictly binary/garbage, or maybe just debug?
                                    // Given we control the client, we expect valid JSON.
                                    let msg_str = std::str::from_utf8(&msg.data).unwrap_or("<invalid utf8>");
                                    error!("Failed to parse input event from '{}': {} | Payload: {}", d_label, e, msg_str);
                                }
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

async fn run_remote_desktop_loop(
    local_track: Arc<TrackLocalStaticRTP>,
    pc_weak: Weak<RTCPeerConnection>,
    mut pli_rx: UnboundedReceiver<()>,
    mut input_rx: UnboundedReceiver<InputEvent>,
) -> Result<()> {
    info!("Starting remote desktop setup...");
    
    // Initialize GStreamer
    gst::init().map_err(|e| anyhow!("Failed to init GStreamer: {}", e))?;

    // Open Portal
    info!("Requesting portal access...");
    let (rdp_proxy, session, stream, fd) = portal::open_portal().await
        .map_err(|e| anyhow!("Failed to open portal: {}", e))?;

    let node_id = stream.pipe_wire_node_id();
    let fd_raw = fd.as_raw_fd();
    info!("Got portal stream: node_id={}, fd={}", node_id, fd_raw);

    let rdp_proxy = Arc::new(rdp_proxy);
    // Keep a local clone for closure if needed, but session is needed for cleanup.
    // Cleanup needs the Arc too if we change close() to use Arc, but PortalInput uses Arc now.
    // wait, if I verify session type.
    let session = Arc::new(session);

    // Spawn Input Event Handler
    let portal_input = PortalInput::new(rdp_proxy.clone(), session.clone());
    tokio::spawn(async move {
        while let Some(event) = input_rx.recv().await {
            let res = match event {
                InputEvent::PointerMotion { dx, dy } => {
                     portal_input.notify_pointer_motion(dx, dy).await
                }
                InputEvent::PointerButton { button, pressed } => {
                    let btn = match button {
                        0 => 272, // BTN_LEFT
                        1 => 274, // BTN_MIDDLE
                        2 => 273, // BTN_RIGHT
                        other => other,
                    };
                     portal_input.notify_pointer_button(btn, pressed).await
                }
                InputEvent::Scroll { steps_x, steps_y } => {
                     portal_input.notify_scroll_discrete(steps_x, steps_y).await
                }
                InputEvent::Key { keycode, pressed } => {
                     portal_input.notify_keyboard_keycode(keycode, pressed).await
                }
            };

            if let Err(e) = res {
                error!("Input injection failed: {:#}", e);
            }
        }
        info!("Input event loop ended");
    });

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

    let queue = gst::ElementFactory::make("queue")
        .build()
        .map_err(|e| anyhow!("Failed to create queue: {}", e))?;
    queue.set_property("max-size-buffers", 1u32);
    queue.set_property("max-size-bytes", 0u32);
    queue.set_property("max-size-time", 0u64);
    queue.set_property_from_str("leaky", "downstream"); // 2 = downstream (drop old)

    let enc = gst::ElementFactory::make("vp8enc")
            .build()
            .map_err(|e| anyhow!("Failed to create enc: {}", e))?;
    enc.set_property_from_str("error-resilient", "partitions");
    enc.set_property("keyframe-max-dist", 2000i32);
    enc.set_property("deadline", 1i64); // Realtime
    enc.set_property("cpu-used", 16i32);
    enc.set_property("threads", 16i32);
    enc.set_property_from_str("end-usage", "cbr");
    enc.set_property("target-bitrate", 2_000_000i32); // 2 Mbps
    enc.set_property("buffer-size", 100i32);
    enc.set_property("buffer-initial-size", 50i32);
    enc.set_property("buffer-optimal-size", 80i32);
    enc.set_property("lag-in-frames", 0i32);
    enc.set_property_from_str("token-partitions", "8"); // 8 partitions, enables multi-threaded decoding on client

    let pay = gst::ElementFactory::make("rtpvp8pay")
            .build()
            .map_err(|e| anyhow!("Failed to create pay: {}", e))?;

    let sink = gst::ElementFactory::make("appsink")
            .build()
            .map_err(|e| anyhow!("Failed to create sink: {}", e))?;
    sink.set_property("sync", false);
    sink.set_property("drop", true);

    pipeline.add_many(&[&src, &conv, &queue, &enc, &pay, &sink])
        .map_err(|e| anyhow!("Failed to add elements: {}", e))?;

    gst::Element::link_many(&[&src, &conv, &queue, &enc, &pay, &sink])
        .map_err(|e| anyhow!("Failed to link elements: {}", e))?;

    let appsink = sink.downcast::<gst_app::AppSink>()
        .map_err(|_| anyhow!("Failed to cast appsink"))?;

    // Start pipeline
    pipeline.set_state(gst::State::Playing)
        .map_err(|e| anyhow!("Failed to set pipeline state to Playing: {}", e))?;
    
    info!("Pipeline playing");

    // Spawn PLI handler
    let pipeline_weak = pipeline.downgrade();
    tokio::spawn(async move {
        while let Some(_) = pli_rx.recv().await {
            if let Some(pipeline) = pipeline_weak.upgrade() {
                let struct_ = gst::Structure::builder("GstForceKeyUnit")
                    .field("all-headers", true)
                    .build();
                let event = gst::event::CustomUpstream::new(struct_);
                pipeline.send_event(event);
            } else {
                break;
            }
        }
    });

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
