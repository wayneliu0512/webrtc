use anyhow::Result;
use axum::{
    Router,
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::services::ServeDir;
use tracing::info;
use webrtc::api::APIBuilder;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MIME_TYPE_VP8, MediaEngine};
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::{RTCRtpCodecCapability, RTPCodecType};
use webrtc::rtp_transceiver::rtp_receiver::RTCRtpReceiver;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_local::{TrackLocal, TrackLocalWriter};
use webrtc::track::track_remote::TrackRemote;

#[tokio::main]
async fn main() -> Result<()> {
    // Configure logging with a default level of INFO if RUST_LOG is not set
    tracing_subscriber::fmt()
        .with_line_number(true)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .fallback_service(ServeDir::new("static"));

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    info!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(mut socket: WebSocket) {
    // Create a MediaEngine object to configure the supported codec
    let mut m = MediaEngine::default();

    // Register default codecs
    m.register_default_codecs().unwrap();

    // Create a InterceptorRegistry. This is the user configurable RTP/RTCP Pipeline.
    // This provides NACKs, RTCP Reports and other features. If you use `webrtc::api::API` the
    // default InterceptorRegistry is used. If you want to register your own interceptors you
    // can do so here.
    let mut registry = Registry::new();

    // Use the default set of Interceptors
    registry = register_default_interceptors(registry, &mut m).unwrap();

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
    let peer_connection = Arc::new(api.new_peer_connection(config).await.unwrap());

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
        .await
        .unwrap();

    // Create a datachannel with label 'data'
    let data_channel = peer_connection
        .create_data_channel("data", None)
        .await
        .unwrap();

    // Set the handler for Peer Connection State
    // This will notify you when the peer has connected/disconnected
    peer_connection.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
        info!("Peer Connection State has changed: {}", s);

        Box::pin(async {})
    }));

    // Register on_track handler for loopback
    let local_track_clone = local_track.clone();
    peer_connection.on_track(Box::new(
        move |track: Arc<TrackRemote>, _receiver: Arc<RTCRtpReceiver>, _transceiver| {
            let local_track = local_track_clone.clone();
            Box::pin(async move {
                info!("Track {} received", track.id());
                if track.kind() == RTPCodecType::Video {
                    info!("Video track received! Starting loopback...");
                    // Loopback loop
                    tokio::spawn(async move {
                        while let Ok((rtp, _)) = track.read_rtp().await {
                            if let Err(e) = local_track.write_rtp(&rtp).await {
                                info!("Failed to write RTP: {}", e);
                                break;
                            }
                        }
                    });
                }
            })
        },
    ));

    // Register channel opening handling
    let d_label = data_channel.label().to_owned();
    let d_id = data_channel.id();
    data_channel.on_open(Box::new(move || {
        info!("Data channel '{}'-'{}' open", d_label, d_id);

        Box::pin(async move {
            // let d2 = d.clone();
            // let d_label2 = d_label.clone();
            // let d_id2 = d_id;
            //
            // // Detach the data channel
            // d2.on_message(Box::new(move |msg: DataChannelMessage| {
            //     let msg_str = String::from_utf8(msg.data.to_vec()).unwrap();
            //     info!("Message from DataChannel '{}'-'{}': '{}'", d_label2, d_id2, msg_str);
            //     Box::pin(async {})
            // }));
        })
    }));

    // Register text message handling
    let d_label = data_channel.label().to_owned();
    let d_clone = data_channel.clone();
    data_channel.on_message(Box::new(move |msg: DataChannelMessage| {
        let msg_str = String::from_utf8(msg.data.to_vec()).unwrap();
        info!("Message from DataChannel '{}': '{}'", d_label, msg_str);

        let d = d_clone.clone();
        Box::pin(async move {
            let reply = format!("Echo from Rust: {}", msg_str);
            let _ = d.send_text(reply).await;
        })
    }));

    // Create an offer to send to the browser
    let offer = peer_connection.create_offer(None).await.unwrap();
    let mut gather_complete = peer_connection.gathering_complete_promise().await;
    peer_connection.set_local_description(offer).await.unwrap();
    let _ = gather_complete.recv().await;

    // Send the offer to the client via WebSocket
    if let Some(local_desc) = peer_connection.local_description().await {
        let json_offer = serde_json::json!({
            "type": "offer",
            "sdp": local_desc.sdp
        });
        socket
            .send(Message::Text(json_offer.to_string()))
            .await
            .unwrap();
    }

    // Handle incoming messages from WebSocket (Answer, ICE Candidates)
    while let Some(msg) = socket.recv().await {
        if let Ok(Message::Text(text)) = msg {
            let json: serde_json::Value = serde_json::from_str(&text).unwrap();
            let type_ = json["type"].as_str().unwrap();

            info!("Received message in signaling: {}", json);
            match type_ {
                "answer" => {
                    let sdp = json["sdp"].as_str().unwrap();
                    let remote_desc = RTCSessionDescription::answer(sdp.to_string()).unwrap();
                    peer_connection
                        .set_remote_description(remote_desc)
                        .await
                        .unwrap();
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
}
