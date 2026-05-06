//! Axum listener, optional browser open, and wiring for the web chat UI.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::config::AppConfig;
use crate::executive::error::{FcpError, Result};
use crate::presentation::{SessionEvent, UserAction};

const EVENT_BACKLOG: usize = 512;

/// Best-effort: launch the system default browser (non-blocking for the parent process).
///
/// Stdio is detached from the Eris process so the browser (often snap Firefox) does not
/// inherit our stderr and flood the terminal with GTK/IBus/Mesa warnings.
fn try_launch_default_browser(url: &str) -> std::io::Result<()> {
    use std::process::{Command, Stdio};

    let attach_detached = |cmd: &mut Command| {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
    };

    if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        attach_detached(&mut c);
        c.spawn()?;
    } else if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", "", url]);
        attach_detached(&mut c);
        c.spawn()?;
    } else if cfg!(unix) {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        attach_detached(&mut c);
        c.spawn()?;
    } else {
        tracing::debug!(
            url = %url,
            "Automatic browser open is not configured for this OS; open the URL manually"
        );
        return Ok(());
    }
    Ok(())
}

/// Shared Axum state for the web chat server.
#[derive(Clone)]
pub struct WebAppState {
    pub events_tx: broadcast::Sender<SessionEvent>,
    pub user_action_tx: mpsc::Sender<UserAction>,
    /// Same token as chat session / SIGINT: cancelling stops Axum and ends `eris chat --web`.
    pub shutdown_token: CancellationToken,
}

/// Run the HTTP server with an **existing** session event broadcast (e.g. presentation multiplexer + Discord).
pub async fn run_web_chat_with_broadcast(
    events_tx: broadcast::Sender<SessionEvent>,
    user_action_tx: mpsc::Sender<UserAction>,
    config: Arc<AppConfig>,
    cancel_token: CancellationToken,
) -> Result<()> {
    let state = WebAppState {
        events_tx: events_tx.clone(),
        user_action_tx,
        shutdown_token: cancel_token.clone(),
    };
    let app: Router = super::router::web_chat_router(state);

    let addr_s = format!("{}:{}", config.web_bind_addr, config.web_port);
    let addr: SocketAddr = addr_s.parse().map_err(|e| {
        FcpError::Config(format!("Invalid web_bind_addr / web_port ({addr_s}): {e}"))
    })?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    let bound = listener.local_addr().map_err(FcpError::Io)?;
    let listen_url = format!("http://{bound}");
    tracing::info!(
        url = %listen_url,
        "Web chat UI listening (open in your browser)"
    );

    if config.web_open_browser {
        let open_url = listen_url.clone();
        tokio::spawn(async move {
            let url_for_blocking = open_url.clone();
            match tokio::task::spawn_blocking(move || try_launch_default_browser(&url_for_blocking))
                .await
            {
                Ok(Ok(())) => tracing::info!(
                    event = "fcp.web.browser_launched",
                    url = %open_url,
                    "Requested default browser for web UI"
                ),
                Ok(Err(e)) => tracing::warn!(
                    event = "fcp.web.browser_launch_failed",
                    error = %e,
                    url = %open_url,
                    "Could not open default browser; open the URL manually"
                ),
                Err(join_err) => tracing::warn!(
                    event = "fcp.web.browser_launch_task_failed",
                    error = %join_err,
                    url = %open_url,
                    "Browser launch task failed"
                ),
            }
        });
    }

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
    let _bridge =
        super::bridge::spawn_presentation_bridge(presentation_rx, bridge_events_tx, bridge_user_tx);

    run_web_chat_with_broadcast(events_tx, user_action_tx, config, cancel_token).await
}
