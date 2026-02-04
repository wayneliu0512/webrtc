use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
};
use futures::SinkExt;
use futures::stream::StreamExt;
use tracing::info;

use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

use crate::webrtc::create_peer_connection;
use edge_lib::protocol::ws_text::signal::{WebRtcSignalChannel, WebRtcSignalChannelType};

pub async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(socket: WebSocket) {
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    // Spawn a task to write messages to the WebSocket
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Err(e) = sender.send(Message::Text(msg)).await {
                info!("Failed to send message: {}", e);
                break;
            }
        }
    });

    // Create a new RTCPeerConnection
    let peer_connection = match create_peer_connection(tx.clone()).await {
        Ok(pc) => pc,
        Err(e) => {
            info!("Failed to create peer connection: {}", e);
            return;
        }
    };

    // Handle incoming messages from WebSocket (Answer, ICE Candidates)
    while let Some(msg) = receiver.next().await {
        if let Ok(Message::Text(text)) = msg {
            let signal: WebRtcSignalChannel = match serde_json::from_str(&text) {
                Ok(s) => s,
                Err(e) => {
                    info!("Received invalid message: {} error: {}", text, e);
                    continue;
                }
            };

            info!("Received message in signaling: {:?}", signal);
            match signal.type_ {
                WebRtcSignalChannelType::Offer => {
                    let sdp = signal.sdp;
                    let remote_desc = RTCSessionDescription::offer(sdp.to_string()).unwrap();
                    peer_connection
                        .set_remote_description(remote_desc)
                        .await
                        .unwrap();

                    let answer = peer_connection.create_answer(None).await.unwrap();
                    let mut gathering_complete = peer_connection.gathering_complete_promise().await;
                    peer_connection.set_local_description(answer).await.unwrap();
                    while let Some(_) = gathering_complete.recv().await {}
                    info!("Gathering complete");

                    if let Some(local_desc) = peer_connection.local_description().await {
                        let json_answer = WebRtcSignalChannel {
                            type_: WebRtcSignalChannelType::Answer,
                            sdp: local_desc.sdp,
                        };
                        let _ = tx.send(serde_json::to_string(&json_answer).unwrap());
                    }
                }
                _ => {}
            }
        }
    }

    if let Err(e) = peer_connection.close().await {
        info!("Failed to close peer connection: {}", e);
    }
}
