use anyhow::{Context, Result, anyhow};
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use tokio::fs;

use ashpd::WindowIdentifier;
use ashpd::desktop::{
    Session,
    screencast::{CursorMode, Screencast, SourceType, Stream},
};

// Assuming edge_lib is available as per Cargo.toml.
// If edge_lib::config::state_dir is not public or available, we might need a fallback.
// But based on ref/portal.rs, it seems to expect it.
// However, to be safe and self-contained, I'll use a local state dir logic or generic temp for now if edge_lib is tricky,
// but sticking to the plan, I should try to use what's there.
// Let's assume we can use standard directories if edge-lib is not easy to import in this new file without more context.
// Actually, let's look at `ref/portal.rs` again. It uses `crate::agent_user_mode::config`.
// This implies `ref/portal.rs` was part of a larger crate structure where `agent_user_mode` existed.
// `src/webrtc.rs` is in `webrtc` crate. `ref` folder is likely from another project or reference.
// `webrtc` crate doesn't seem to have `agent_user_mode`.
// So I should implement a simple `get_state_dir` or just store token in `/tmp` or XDG_STATE_HOME for now.
// I'll use `dirs` or just `std::env::temp_dir()` for simplicity unless `ashpd` has helper.
// `ashpd` has no state helper. I'll implement a simple one.

fn state_dir() -> Result<PathBuf> {
    let mut path = std::env::temp_dir();
    path.push("webrtc-remote-desktop");
    if !path.exists() {
        std::fs::create_dir_all(&path)?;
    }
    Ok(path)
}

fn restore_token_path() -> Result<PathBuf> {
    state_dir().map(|d| d.join("screencast_restore_token"))
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

/// Opens the xdg-desktop-portal Screencast UI and returns the selected PipeWire
/// stream plus the PipeWire remote fd, and the portal session handle.
///
/// Important: callers should call `session.close().await` when finished, otherwise
/// the portal may keep the screencast session alive longer than expected.
pub async fn open_portal() -> Result<(Session<'static, Screencast<'static>>, Stream, OwnedFd)> {
    let proxy = Screencast::new()
        .await
        .with_context(|| "'Screencast::new' failed")?;

    let session = proxy
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

    proxy
        .select_sources(
            &session,
            CursorMode::Metadata, // Changed to Metadata to allow client to render cursor if needed, or Embedded. Embedded is safer for video. Ref used Embedded.
            SourceType::Monitor | SourceType::Window,
            false,
            restore_token_opt,
            ashpd::desktop::PersistMode::ExplicitlyRevoked, // Correct enum? Ref used `ashpd::desktop::PersistMode::ExplicitlyRevoked`
        )
        .await
        .with_context(|| "'proxy.select_sources' failed")?;

    let response = proxy
        .start(&session, &WindowIdentifier::default())
        .await
        .with_context(|| "'proxy.start' failed")?
        .response()
        .with_context(|| "'response' failed")?;

    if let Some(token) = response.restore_token() {
        save_restore_token(token)
            .await
            .with_context(|| "'save_restore_token' failed")?;
    }

    let stream = response
        .streams()
        .first()
        .ok_or(anyhow!("No stream found or selected"))?
        .to_owned();

    let fd = proxy
        .open_pipe_wire_remote(&session)
        .await
        .with_context(|| "'proxy.open_pipe_wire_remote' failed")?;

    Ok((session, stream, fd))
}
