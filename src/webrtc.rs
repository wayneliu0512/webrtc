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
use webrtc::rtp_transceiver::rtp_codec::{RTCRtpCodecCapability, RTPCodecType};
use webrtc::rtp_transceiver::rtp_receiver::RTCRtpReceiver;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_local::{TrackLocal, TrackLocalWriter};
use webrtc::track::track_remote::TrackRemote;

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
