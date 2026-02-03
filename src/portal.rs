use anyhow::{Context, Result, anyhow};
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use tokio::fs;

use ashpd::WindowIdentifier;
use ashpd::desktop::{
    PersistMode, Session,
    clipboard::Clipboard,
    remote_desktop::{Axis, DeviceType, KeyState, RemoteDesktop},
    screencast::{CursorMode, Screencast, SourceType, Stream},
};

fn state_dir() -> Result<PathBuf> {
    let mut path = std::env::temp_dir();
    path.push("webrtc-remote-desktop");
    if !path.exists() {
        std::fs::create_dir_all(&path)?;
    }
    Ok(path)
}

fn restore_token_path() -> Result<PathBuf> {
    state_dir().map(|d| d.join("remote_desktop_restore_token"))
}

async fn load_restore_token() -> Result<String> {
    let path = restore_token_path()?;
    if !path.exists() {
        return Ok("".to_string());
    }
    let content = fs::read_to_string(path).await?;
    let token = content.trim().to_string();
    Ok(token)
}

async fn save_restore_token(token: &str) -> Result<()> {
    match restore_token_path() {
        Ok(path) => {
            fs::write(&path, &format!("{token}\n"))
                .await
                .with_context(|| "Failed to write to file")?;
        }
        Err(e) => {
            return Err(e);
        }
    }
    Ok(())
}

/// Opens the xdg-desktop-portal RemoteDesktop UI and returns the selected PipeWire
/// stream plus the PipeWire remote fd, and the portal session handle.
///
/// Important: callers should call `session.close().await` when finished, otherwise
/// the portal may keep the remote desktop session alive longer than expected.
pub async fn open_portal() -> Result<(Session<'static, RemoteDesktop<'static>>, Stream, OwnedFd)> {
    let rdp_proxy = RemoteDesktop::new()
        .await
        .with_context(|| "'RemoteDesktop::new' failed")?;

    let srncast_proxy = Screencast::new()
        .await
        .with_context(|| "'Screencast::new' failed")?;

    let session = rdp_proxy
        .create_session()
        .await
        .with_context(|| "'proxy.create_session' failed")?;

    let restore_token = load_restore_token()
        .await
        .with_context(|| "'load_restore_token' failed")?;

    let restore_token_opt = if restore_token.is_empty() {
        None
    } else {
        Some(restore_token.as_str())
    };

    srncast_proxy
        .select_sources(
            &session,
            CursorMode::Embedded,
            SourceType::Monitor | SourceType::Window,
            false,
            None,
            ashpd::desktop::PersistMode::DoNot,
        )
        .await
        .with_context(|| "'srn_cast_proxy.select_sources' failed")?;

    rdp_proxy
        .select_devices(
            &session,
            DeviceType::Keyboard | DeviceType::Pointer,
            restore_token_opt,
            PersistMode::ExplicitlyRevoked,
        )
        .await
        .with_context(|| "'proxy.select_devices' failed")?;

    let _ = match Clipboard::new().await {
        Ok(clipboard) => match clipboard.request(&session).await {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("Clipboard request failed: {:#}", e);
                false
            }
        },
        Err(e) => {
            tracing::warn!("Failed to create clipboard portal: {:#}", e);
            false
        }
    };

    let remote_desktop_response = rdp_proxy
        .start(&session, &WindowIdentifier::default())
        .await
        .with_context(|| "'proxy.start' failed")?
        .response()
        .map_err(|e| anyhow!("RemoteDesktop start response error: {:#}", e))?;

    let stream = remote_desktop_response
        .streams()
        .ok_or(anyhow!("No streams available in RemoteDesktop response"))?
        .first()
        .ok_or(anyhow!("No stream found or selected"))?
        .to_owned();

    let fd = srncast_proxy
        .open_pipe_wire_remote(&session)
        .await
        .with_context(|| "'srncast_proxy.open_pipe_wire_remote' failed")?;

    if let Some(token) = remote_desktop_response.restore_token() {
        save_restore_token(token)
            .await
            .with_context(|| "'save_restore_token' failed")?;
    }

    Ok((session, stream, fd))
}
