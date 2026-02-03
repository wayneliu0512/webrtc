use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
};
use futures::SinkExt;
use futures::stream::StreamExt;
use tracing::info;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

use crate::webrtc::create_peer_connection;

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
            let json: serde_json::Value = serde_json::from_str(&text).unwrap();
            let type_ = match json["type"].as_str() {
                Some(t) => t,
                None => {
                    info!("Received message without type: {}", json);
                    continue;
                }
            };

            info!("Received message in signaling: {}", json);
            match type_ {
                "offer" => {
                    let sdp = json["sdp"].as_str().unwrap();
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
                        let json_answer = serde_json::json!({
                            "type": "answer",
                            "sdp": local_desc.sdp
                        });
                        let _ = tx.send(json_answer.to_string());
                    }
                }
                "candidate" => {
                    if let Some(candidate) = json["candidate"].as_object() {
                        let candidate_init = serde_json::from_value::<RTCIceCandidateInit>(
                            serde_json::Value::Object(candidate.clone()),
                        )
                        .unwrap();
                        peer_connection
                            .add_ice_candidate(candidate_init)
                            .await
                            .unwrap();
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
