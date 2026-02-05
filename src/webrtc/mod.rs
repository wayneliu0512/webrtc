mod callbacks;
mod clipboard;
mod gstreamer;
mod input;

use ::gstreamer as gst;
use gst::prelude::*;

use anyhow::{Result, anyhow};
use std::os::fd::AsRawFd;
use std::sync::{Arc, Weak};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tracing::{error, info};
use webrtc::api::APIBuilder;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MIME_TYPE_VP8, MediaEngine};
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use webrtc::rtp::packet::Packet;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_local::{TrackLocal, TrackLocalWriter};
use webrtc::util::Unmarshal;

use crate::portal;
use edge_lib::protocol::webrtc::data_channel::{ClipboardEvent, DataChannelMsg, InputEvent};

use self::callbacks::{
    setup_connection_state_handling, setup_data_channel_handling, setup_ice_handling,
    setup_track_handling,
};
use self::clipboard::clipboard_loop;
use self::gstreamer::{build_gstreamer_pipeline, spawn_pli_handler};
use self::input::{PortalInput, spawn_input_handler};

/// Creates a new WebRTC PeerConnection with remote desktop capabilities.
pub async fn create_peer_connection(tx: UnboundedSender<String>) -> Result<Arc<RTCPeerConnection>> {
    // 1. Basic WebRTC API and Configuration setup
    let api = create_api()?;
    let config = RTCConfiguration {
        // ice_servers: vec![],
        ice_servers: vec![RTCIceServer {
            urls: vec!["stun:stun.cloudflare.com:3478".to_owned()],
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
                if packet
                    .as_any()
                    .downcast_ref::<PictureLossIndication>()
                    .is_some()
                {
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
    setup_track_handling(&peer_connection);
    setup_data_channel_handling(&peer_connection, input_tx, clipboard_cmd_tx, dc_out_rx);
    setup_ice_handling(&peer_connection, tx);
    setup_connection_state_handling(&peer_connection);

    // 5. Spawn Remote Desktop Task
    // We pass a Weak reference to the PC so the remote desktop loop can stop if PC is dropped/closed.
    let pc_weak = Arc::downgrade(&peer_connection);
    tokio::spawn(async move {
        if let Err(e) = run_remote_desktop_loop(
            local_track,
            pc_weak,
            pli_rx,
            input_rx,
            clipboard_cmd_rx,
            dc_out_tx,
        )
        .await
        {
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
    let (rdp_proxy, clipboard_proxy, session, stream, fd) = portal::open_portal()
        .await
        .map_err(|e| anyhow!("Failed to open portal: {}", e))?;
    let rdp_proxy = Arc::new(rdp_proxy);
    let clipboard_proxy = Arc::new(clipboard_proxy);
    let session = Arc::new(session);

    // Spawn Input Event Handler
    let portal_input = PortalInput::new(rdp_proxy, session.clone());
    spawn_input_handler(portal_input, input_rx);

    // Create shutdown signal for clipboard loop
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Spawn Clipboard Loop
    tokio::spawn(clipboard_loop(
        clipboard_proxy,
        session.clone(),
        dc_out_tx,
        clipboard_cmd_rx,
        shutdown_rx,
    ));

    // Build Pipeline and play
    let (pipeline, appsink) = build_gstreamer_pipeline(fd.as_raw_fd(), stream.pipe_wire_node_id())?;
    pipeline
        .set_state(gst::State::Playing)
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
