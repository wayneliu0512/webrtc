use anyhow::Result;
use axum::{Router, routing::get};
use std::net::SocketAddr;
use tower_http::services::ServeDir;
use tracing::info;

mod handlers;
mod portal;
mod webrtc;

use handlers::ws_handler;

#[tokio::main]
async fn main() -> Result<()> {
    // Configure logging with a default level of INFO if RUST_LOG is not set
    tracing_subscriber::fmt()
        .with_line_number(true)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "my_webrtc=debug".into()),
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
