use edge_lib::protocol::webrtc::data_channel::{ClipboardEvent, DataChannelMsg, InputEvent};
use std::sync::Arc;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tracing::{error, info};
use webrtc::data_channel::RTCDataChannel;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::ice_transport::ice_candidate::RTCIceCandidate;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::rtp_transceiver::rtp_receiver::RTCRtpReceiver;
use webrtc::track::track_remote::TrackRemote;

/// Sets up Data Channel handlers.
pub fn setup_data_channel_handling(
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
                                    let msg_str =
                                        std::str::from_utf8(&msg.data).unwrap_or("<invalid utf8>");
                                    error!(
                                        "Failed to parse input event from '{}': {} | Payload: {}",
                                        d_label, e, msg_str
                                    );
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
pub fn setup_ice_handling(pc: &Arc<RTCPeerConnection>, tx: UnboundedSender<String>) {
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
pub fn setup_connection_state_handling(pc: &Arc<RTCPeerConnection>) {
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

pub fn setup_track_handling(pc: &Arc<RTCPeerConnection>) {
    pc.on_track(Box::new(
        move |track: Arc<TrackRemote>, _receiver: Arc<RTCRtpReceiver>, _transceiver| {
            Box::pin(async move {
                info!("Track {} received", track.id());
            })
        },
    ));
}
