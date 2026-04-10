//! Localhost web UI for chat (`eris chat --web`): Axum + SSE + minimal JS.

mod bridge;
mod handlers;
mod router;
mod sse;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::config::AppConfig;
use crate::executive::error::{FcpError, Result};
use crate::presentation::{SessionEvent, UserAction};

const EVENT_BACKLOG: usize = 512;

/// Shared Axum state for the web chat server.
#[derive(Clone)]
pub struct WebAppState {
    pub events_tx: broadcast::Sender<SessionEvent>,
    pub user_action_tx: mpsc::Sender<UserAction>,
}

/// Run the HTTP server until `cancel_token` is cancelled or the listener fails.
pub async fn run_web_chat(
    presentation_rx: mpsc::Receiver<SessionEvent>,
    user_action_tx: mpsc::Sender<UserAction>,
    config: Arc<AppConfig>,
    cancel_token: CancellationToken,
) -> Result<()> {
    let (events_tx, _) = broadcast::channel::<SessionEvent>(EVENT_BACKLOG);
    let bridge_user_tx = user_action_tx.clone();
    let bridge_events_tx = events_tx.clone();
    let _bridge = bridge::spawn_presentation_bridge(presentation_rx, bridge_events_tx, bridge_user_tx);

    let state = WebAppState {
        events_tx: events_tx.clone(),
        user_action_tx,
    };
    let app: Router = router::web_chat_router(state);

    let addr_s = format!("{}:{}", config.web_bind_addr, config.web_port);
    let addr: SocketAddr = addr_s.parse().map_err(|e| {
        FcpError::Config(format!(
            "Invalid web_bind_addr / web_port ({addr_s}): {e}"
        ))
    })?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    let bound = listener.local_addr().map_err(FcpError::Io)?;
    tracing::info!(
        url = %format!("http://{bound}"),
        "Web chat UI listening (open in your browser)"
    );

    let cancel_serve = cancel_token.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            cancel_serve.cancelled().await;
            tracing::info!("Web server graceful shutdown requested");
        })
        .await
        .map_err(|e| FcpError::NetworkFault(format!("Web server error: {e}")))?;

    Ok(())
}
