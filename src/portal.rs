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
pub async fn open_portal() -> Result<(
    RemoteDesktop<'static>,
    Session<'static, RemoteDesktop<'static>>,
    Stream,
    OwnedFd,
)> {
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

    Ok((rdp_proxy, session, stream, fd))
}

#[derive(Clone)]
pub struct PortalInput {
    proxy: std::sync::Arc<RemoteDesktop<'static>>,
    session: std::sync::Arc<Session<'static, RemoteDesktop<'static>>>,
}

impl PortalInput {
    pub fn new(
        proxy: std::sync::Arc<RemoteDesktop<'static>>,
        session: std::sync::Arc<Session<'static, RemoteDesktop<'static>>>,
    ) -> Self {
        Self { proxy, session }
    }

    pub async fn notify_pointer_motion(&self, dx: i32, dy: i32) -> Result<()> {
        self.proxy
            .notify_pointer_motion(&self.session, dx as f64, dy as f64)
            .await
            .with_context(|| "'notify_pointer_motion' failed")?;
        Ok(())
    }

    pub async fn notify_pointer_button(&self, button: u32, pressed: bool) -> Result<()> {
        let state = if pressed {
            KeyState::Pressed
        } else {
            KeyState::Released
        };
        self.proxy
            .notify_pointer_button(
                &self.session,
                button
                    .try_into()
                    .map_err(|_| anyhow!("button out of range"))?,
                state,
            )
            .await
            .with_context(|| "'notify_pointer_button' failed")?;
        Ok(())
    }

    pub async fn notify_keyboard_keycode(&self, keycode: u32, pressed: bool) -> Result<()> {
        let state = if pressed {
            KeyState::Pressed
        } else {
            KeyState::Released
        };
        self.proxy
            .notify_keyboard_keycode(
                &self.session,
                keycode
                    .try_into()
                    .map_err(|_| anyhow!("keycode out of range"))?,
                state,
            )
            .await
            .with_context(|| "'notify_keyboard_keycode' failed")?;
        Ok(())
    }

    pub async fn notify_scroll_discrete(&self, steps_x: i32, steps_y: i32) -> Result<()> {
        if steps_x != 0 {
            self.proxy
                .notify_pointer_axis_discrete(&self.session, Axis::Horizontal, steps_x)
                .await
                .with_context(|| "'notify_pointer_axis_discrete(Horizontal)' failed")?;
        }
        if steps_y != 0 {
            self.proxy
                .notify_pointer_axis_discrete(&self.session, Axis::Vertical, steps_y)
                .await
                .with_context(|| "'notify_pointer_axis_discrete(Vertical)' failed")?;
        }
        Ok(())
    }
}
