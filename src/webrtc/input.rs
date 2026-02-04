use anyhow::{Context, Result, anyhow};
use ashpd::desktop::{
    Session,
    remote_desktop::{Axis, KeyState, RemoteDesktop},
};
use edge_lib::protocol::webrtc::data_channel::InputEvent;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::{error, info};

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

pub fn spawn_input_handler(portal_input: PortalInput, mut input_rx: UnboundedReceiver<InputEvent>) {
    tokio::spawn(async move {
        while let Some(event) = input_rx.recv().await {
            let res = match event {
                InputEvent::PointerMotion { dx, dy } => {
                    portal_input.notify_pointer_motion(dx, dy).await
                }
                InputEvent::PointerButton { button, pressed } => {
                    let btn = match button {
                        0 => 272, // BTN_LEFT
                        1 => 274, // BTN_MIDDLE
                        2 => 273, // BTN_RIGHT
                        other => other,
                    };
                    portal_input.notify_pointer_button(btn, pressed).await
                }
                InputEvent::Scroll { steps_x, steps_y } => {
                    portal_input.notify_scroll_discrete(steps_x, steps_y).await
                }
                InputEvent::Key { keycode, pressed } => {
                    portal_input.notify_keyboard_keycode(keycode, pressed).await
                }
            };

            if let Err(e) = res {
                error!("Input injection failed: {:#}", e);
            }
        }
        info!("Input event loop ended");
    });
}
