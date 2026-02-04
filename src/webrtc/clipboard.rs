use anyhow::{Result, anyhow};
use ashpd::desktop::clipboard::Clipboard;
use edge_lib::protocol::webrtc::data_channel::{ClipboardEvent, DataChannelMsg};
use futures::StreamExt;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tracing::{error, info};

const CLIPBOARD_MIME_TEXT: &str = "text/plain;charset=utf-8";
const CLIPBOARD_MIME_TEXT_FALLBACK: &str = "text/plain";

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
    let text =
        String::from_utf8(buf).map_err(|e| anyhow!("Clipboard text is not valid UTF-8: {}", e))?;
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

pub async fn clipboard_loop(
    clipboard: Arc<Clipboard<'static>>,
    session: Arc<
        ashpd::desktop::Session<'static, ashpd::desktop::remote_desktop::RemoteDesktop<'static>>,
    >,
    dc_out_tx: UnboundedSender<DataChannelMsg>,
    mut clipboard_cmd_rx: UnboundedReceiver<ClipboardEvent>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
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
