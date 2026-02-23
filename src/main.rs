use ::webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use anyhow::Result;
use std::io::BufRead;
use tracing::info;

mod portal;
mod webrtc;

use crate::webrtc::create_peer_connection;
use edge_lib::protocol::ws_text::signal::{WebRtcSignalChannel, WebRtcSignalChannelType};

#[tokio::main]
async fn main() -> Result<()> {
    // Configure logging with a default level of INFO if RUST_LOG is not set
    tracing_subscriber::fmt()
        .with_line_number(true)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "my_webrtc=debug".into()),
        )
        .init();

    info!("Starting WebRTC in Manual Signaling Mode");

    // Create a channel for the peer connection to send messages (e.g. candidates)
    // In manual mode, we probably just ignore subsequent candidates if we wait for gathering complete.
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    // Create PeerConnection
    let peer_connection = create_peer_connection(tx).await?;

    info!("Please paste the OFFER SDP json (single line) and press Enter:");

    // Read from stdin
    // Use tokio::task::spawn_blocking or just std::io since we are in main and before the main loop
    let mut line = String::new();
    let stdin = std::io::stdin();
    stdin.lock().read_line(&mut line)?;

    // Parse Offer
    // Expecting JSON: {"type": "offer", "sdp": "..."}
    let signal: WebRtcSignalChannel = serde_json::from_str(&line.trim())?;

    if let WebRtcSignalChannelType::Offer = signal.type_ {
        info!("Received Offer, setting remote description...");
        let remote_desc = RTCSessionDescription::offer(signal.sdp.clone())?;
        peer_connection.set_remote_description(remote_desc).await?;

        info!("Creating Answer...");
        let answer = peer_connection.create_answer(None).await?;

        // Wait for ICE gathering to complete so we have a complete SDP
        let mut gathering_complete = peer_connection.gathering_complete_promise().await;
        peer_connection.set_local_description(answer).await?;

        info!("Waiting for ICE gathering to complete...");
        let _ = gathering_complete.recv().await;
        info!("ICE Gathering complete");

        if let Some(local_desc) = peer_connection.local_description().await {
            let json_answer = WebRtcSignalChannel {
                type_: WebRtcSignalChannelType::Answer,
                sdp: local_desc.sdp,
            };
            info!("Here is the ANSWER SDP:");
            println!("{}", serde_json::to_string(&json_answer)?);
        }
    } else {
        info!("Expected Offer, got {:?}", signal.type_);
        return Ok(());
    }

    // Keep the process alive to maintain the connection
    info!("WebRTC session established. Press Ctrl+C to exit.");
    tokio::signal::ctrl_c().await?;

    // Close cleanly
    if let Err(e) = peer_connection.close().await {
        info!("Failed to close peer connection: {}", e);
    }

    Ok(())
}
