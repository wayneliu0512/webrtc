use anyhow::{anyhow, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use std::os::fd::{AsRawFd, RawFd};
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
use ashpd::desktop::clipboard::Clipboard;
use futures::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const CLIPBOARD_MIME_TEXT: &str = "text/plain;charset=utf-8";
const CLIPBOARD_MIME_TEXT_FALLBACK: &str = "text/plain";


use crate::portal::{self, PortalInput};
use edge_lib::protocol::webrtc::data_channel::{DataChannelMsg, InputEvent, ClipboardEvent};

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
    let (clipboard_cmd_tx, clipboard_cmd_rx) = unbounded_channel::<ClipboardEvent>();
    // Channel for sending messages OUT to the data channel (e.g. from clipboard loop to frontend)
    let (dc_out_tx, dc_out_rx) = unbounded_channel::<DataChannelMsg>();

    // 4. Register Handlers
    // Handle incoming tracks (Placeholder for now)
    peer_connection.on_track(Box::new(
        move |track: Arc<TrackRemote>, _receiver: Arc<RTCRtpReceiver>, _transceiver| {
            Box::pin(async move {
                info!("Track {} received", track.id());
            })
        },
    ));

    setup_data_channel_handling(&peer_connection, input_tx, clipboard_cmd_tx, dc_out_rx);
    setup_ice_handling(&peer_connection, tx);
    setup_connection_state_handling(&peer_connection);

    // 5. Spawn Remote Desktop Task
    // We pass a Weak reference to the PC so the remote desktop loop can stop if PC is dropped/closed.
    let pc_weak = Arc::downgrade(&peer_connection);
    tokio::spawn(async move {
        if let Err(e) = run_remote_desktop_loop(local_track, pc_weak, pli_rx, input_rx, clipboard_cmd_rx, dc_out_tx).await {
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
/// Sets up Data Channel handlers.
fn setup_data_channel_handling(
    pc: &Arc<RTCPeerConnection>,
    input_tx: UnboundedSender<InputEvent>,
    clipboard_cmd_tx: UnboundedSender<ClipboardEvent>,
    dc_out_rx: UnboundedReceiver<DataChannelMsg>,
) {
    // We wrap dc_out_rx in an Arc<Mutex<Option<...>>> so we can claim it in the callback
    // The first data channel to open will claim it and become the output channel.
    let dc_out_rx = Arc::new(std::sync::Mutex::new(Some(dc_out_rx)));

    pc.on_data_channel(Box::new(move |d: Arc<RTCDataChannel>| {
        let d_label = d.label().to_owned();
        let d_id = d.id();
        info!("New DataChannel {} {}", d_label, d_id);

        let input_tx = input_tx.clone();
        let clipboard_cmd_tx = clipboard_cmd_tx.clone();
        let dc_out_rx = dc_out_rx.clone();

        Box::pin(async move {
            let d_label_clone = d_label.clone();
            let d_clone = d.clone();
            
            d.on_open(Box::new(move || {
                info!("Data channel '{}'-'{}' open", d_label_clone, d_id);
                
                let d_clone = d_clone.clone();
                let d_label_clone = d_label_clone.clone();
                let input_tx = input_tx.clone();
                let clipboard_cmd_tx = clipboard_cmd_tx.clone();
                
                // Try to claim the output receiver
                let rx_opt = dc_out_rx.lock().unwrap().take();

                Box::pin(async move {
                    let d_label_inner = d_label_clone.clone();
                    
                    // Spawn output task if we claimed the receiver
                    if let Some(mut rx) = rx_opt {
                         let d_sender = d_clone.clone();
                         tokio::spawn(async move {
                             info!("Starting DataChannel writer loop");
                             while let Some(msg) = rx.recv().await {
                                 if let Ok(json) = serde_json::to_string(&msg) {
                                     if let Err(e) = d_sender.send_text(json).await {
                                         error!("Failed to send data channel msg: {}", e);
                                     }
                                 }
                             }
                             info!("DataChannel writer loop ended");
                         });
                    }

                    d_clone.on_message(Box::new(move |msg: DataChannelMessage| {
                        let input_tx = input_tx.clone();
                        let clipboard_cmd_tx = clipboard_cmd_tx.clone(); // Use this!
                        let d_label = d_label_inner.clone();

                        Box::pin(async move {
                            match serde_json::from_slice::<DataChannelMsg>(&msg.data) {
                                Ok(DataChannelMsg::Clipboard(event)) => {
                                    if let Err(e) = clipboard_cmd_tx.send(event) {
                                        error!("Failed to forward clipboard event: {}", e);
                                    }
                                }
                                Ok(DataChannelMsg::InputEvent(event)) => {
                                    if let Err(e) = input_tx.send(event) {
                                        error!("Failed to forward input event: {}", e);
                                    }
                                }
                                Err(e) => {
                                    // Log only if it's not strictly binary/garbage, or maybe just debug?
                                    // Given we control the client, we expect valid JSON byteload.
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
    pli_rx: UnboundedReceiver<()>,
    input_rx: UnboundedReceiver<InputEvent>,
    clipboard_cmd_rx: UnboundedReceiver<ClipboardEvent>,
    dc_out_tx: UnboundedSender<DataChannelMsg>,
) -> Result<()> {
    info!("Starting remote desktop setup...");
    
    // Initialize GStreamer
    gst::init().map_err(|e| anyhow!("Failed to init GStreamer: {}", e))?;

    // Open Portal
    info!("Requesting portal access...");
    let (rdp_proxy, session, stream, fd) = portal::open_portal().await
        .map_err(|e| anyhow!("Failed to open portal: {}", e))?;
    let rdp_proxy = Arc::new(rdp_proxy);
    let session = Arc::new(session);

    // Spawn Input Event Handler
    let portal_input = PortalInput::new(rdp_proxy.clone(), session.clone());
    spawn_input_handler(portal_input, input_rx);
    
    // Create shutdown signal for clipboard loop
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    
    // Spawn Clipboard Loop
    // We reuse the rdp_proxy (RemoteDesktop portal handle)
    // Note: The original 'remote' struct in 'ref/remote_desktop.rs' bundled session and proxy. 
    // Here we have them separate. Adapt clipboard_loop accordingly.
    tokio::spawn(clipboard_loop(rdp_proxy, session.clone(), dc_out_tx, clipboard_cmd_rx, shutdown_rx));


    // Build Pipeline and play
    let (pipeline, appsink) = build_gstreamer_pipeline(fd.as_raw_fd(), stream.pipe_wire_node_id())?;
    pipeline.set_state(gst::State::Playing)
        .map_err(|e| anyhow!("Failed to set pipeline state to Playing: {}", e))?;
    info!("Pipeline playing");

    // Spawn PLI handler
    spawn_pli_handler(pipeline.clone(), pli_rx);

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
    let _ = shutdown_tx.send(true);
    let _ = pipeline.set_state(gst::State::Null);
    let _ = session.close().await;
    
    Ok(())
}

fn spawn_input_handler(
    portal_input: PortalInput,
    mut input_rx: UnboundedReceiver<InputEvent>,
) {
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
}

fn build_gstreamer_pipeline(
    fd_raw: RawFd,
    node_id: u32,
) -> Result<(gst::Pipeline, gst_app::AppSink)> {
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
        
    Ok((pipeline, appsink))
}

fn spawn_pli_handler(pipeline: gst::Pipeline, mut pli_rx: UnboundedReceiver<()>) {
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
}

// Helpers adapting from ref/remote_desktop.rs

async fn read_fd_text(fd: std::os::fd::OwnedFd) -> Result<String> {
    let std_file = std::fs::File::from(fd);
    let mut file = tokio::fs::File::from_std(std_file);
    let mut buf = Vec::new();
    let mut attempts = 0u8;
    loop {
        match file.read_to_end(&mut buf).await {
            Ok(_) => break,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && attempts < 5 => {
                attempts += 1;
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                continue;
            }
            Err(e) => return Err(anyhow!("Failed to read clipboard data: {}", e)),
        }
    }
    let text = String::from_utf8(buf).map_err(|e| anyhow!("Clipboard text is not valid UTF-8: {}", e))?;
    Ok(text)
}

async fn write_fd_text(fd: std::os::fd::OwnedFd, text: &str) -> Result<()> {
    let std_file = std::fs::File::from(fd);
    let mut file = tokio::fs::File::from_std(std_file);
    file.write_all(text.as_bytes())
        .await
        .map_err(|e| anyhow!("Failed to write clipboard data: {}", e))?;
    Ok(())
}

async fn clipboard_loop(
    rdp_proxy: Arc<ashpd::desktop::remote_desktop::RemoteDesktop<'static>>,
    session: Arc<ashpd::desktop::Session<'static, ashpd::desktop::remote_desktop::RemoteDesktop<'static>>>,
    dc_out_tx: UnboundedSender<DataChannelMsg>,
    mut clipboard_cmd_rx: UnboundedReceiver<ClipboardEvent>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    let clipboard = match Clipboard::new().await {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to create clipboard portal: {:#}", e);
            return;
        }
    };

    let mut owner_changed = match clipboard.receive_selection_owner_changed().await {
        Ok(stream) => stream.boxed(),
        Err(e) => {
            error!("Failed to receive clipboard owner changes: {:#}", e);
            return;
        }
    };

    let mut transfer_stream = match clipboard.receive_selection_transfer().await {
        Ok(stream) => stream.boxed(),
        Err(e) => {
            error!("Failed to receive clipboard transfer signals: {:#}", e);
            return;
        }
    };

    let mut current_text: String = String::new();
    let mut pending_text: Option<String> = None;

    info!("Clipboard loop started");

    loop {
        if *shutdown_rx.borrow() {
            break;
        }
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            Some(cmd) = clipboard_cmd_rx.recv() => {
                match cmd {
                    ClipboardEvent::SetText(text) => {
                        info!("Clipboard SetText: (len={})", text.len());
                        let mime_types = [CLIPBOARD_MIME_TEXT, CLIPBOARD_MIME_TEXT_FALLBACK];
                        if let Err(e) = clipboard.set_selection(&session, &mime_types).await {
                            error!("Failed to set clipboard selection: {:#}", e);
                            continue;
                        }
                        // Update cache optimistically
                        current_text = text.clone();
                        pending_text = Some(text);
                    }
                    ClipboardEvent::GetText => {
                        info!("Clipboard GetText: returning cached text (len={})", current_text.len());
                        let msg = DataChannelMsg::Clipboard(ClipboardEvent::Text(current_text.clone()));
                        if dc_out_tx.send(msg).is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            Some((session, change)) = owner_changed.next() => {
                if change.session_is_owner() == Some(true) {
                    info!("Clipboard owner changed: we are owner");
                    continue;
                }

                let mime_types = change.mime_types();
                let candidates = if mime_types.is_empty() {
                     vec![CLIPBOARD_MIME_TEXT, CLIPBOARD_MIME_TEXT_FALLBACK]
                } else {
                     let mut c = Vec::new();
                     if mime_types.iter().any(|m| m == CLIPBOARD_MIME_TEXT) {
                         c.push(CLIPBOARD_MIME_TEXT);
                     } else if mime_types.iter().any(|m| m == CLIPBOARD_MIME_TEXT_FALLBACK) {
                         c.push(CLIPBOARD_MIME_TEXT_FALLBACK);
                     }
                     c
                };

                if candidates.is_empty() {
                    continue;
                }

                let mut text = None;
                for mime_type in candidates {
                    // Use the session provided by the signal for reading
                    match clipboard.selection_read(&session, mime_type).await {
                        Ok(fd) => {
                             // Note: ashpd returns OwnedFd
                            match read_fd_text(fd.into()).await {
                                Ok(value) => {
                                    text = Some(value);
                                    break;
                                }
                                Err(e) => {
                                    error!("Failed to read clipboard text locally ({mime_type}): {:#}", e);
                                }
                            }
                        }
                        Err(e) => {
                            error!("Failed to request clipboard selection locally ({mime_type}): {:#}", e);
                        }
                    }
                }

                if let Some(t) = text {
                    info!("Clipboard updated locally: (len={})", t.len());
                    current_text = t;
                     // Optional: notify remote immediately? 
                     // The original impl didn't push to remote, only cached it for GetText.
                     // But if we want auto-sync, we should enable this:
                     // let msg = DataChannelMsg::Clipboard(ClipboardEvent::Text(current_text.clone()));
                     // let _ = dc_out_tx.send(msg);
                }
            }
            Some((session, mime_type, serial)) = transfer_stream.next() => {
                let Some(text) = pending_text.as_ref() else {
                    info!("Clipboard transfer: no pending text");
                    let _ = clipboard.selection_write_done(&session, serial, false).await;
                    continue;
                };
                if mime_type != CLIPBOARD_MIME_TEXT && mime_type != CLIPBOARD_MIME_TEXT_FALLBACK {
                    info!("Clipboard transfer: invalid mime type: {}", mime_type);
                    let _ = clipboard.selection_write_done(&session, serial, false).await;
                    continue;
                }
                info!("Clipboard transfer: writing text (len={})", text.len());
                match clipboard.selection_write(&session, serial).await {
                    Ok(fd) => {
                        let result = write_fd_text(fd.into(), text).await;
                        let success = result.is_ok();
                        if let Err(e) = result {
                            error!("Failed to write clipboard transfer: {:#}", e);
                        }
                        if let Err(e) = clipboard.selection_write_done(&session, serial, success).await {
                            error!("Failed to write clipboard transfer done: {:#}", e);
                        }
                    }
                    Err(e) => {
                        error!("Failed to open clipboard transfer fd: {:#}", e);
                        let _ = clipboard.selection_write_done(&session, serial, false).await;
                    }
                }
            }
        }
    }
    info!("Clipboard loop ended");
}
