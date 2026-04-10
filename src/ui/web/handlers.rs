//! HTTP handlers for the web chat shell and API.

use askama::Template;
use askama_web::WebTemplate;
use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::presentation::UserAction;

use super::WebAppState;

#[derive(Template, WebTemplate)]
#[template(path = "chat.html")]
pub struct ChatShell;

pub async fn chat_shell() -> ChatShell {
    ChatShell
}

pub async fn chat_js() -> Response {
    match Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/javascript; charset=utf-8")
        .body(Body::from(include_str!("assets/chat.js")))
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "failed to build chat.js response");
            Response::new(Body::from("/* asset error */"))
        }
    }
}

pub async fn post_action(
    State(state): State<WebAppState>,
    Json(action): Json<UserAction>,
) -> impl IntoResponse {
    match state.user_action_tx.try_send(action) {
        Ok(()) => {
            tracing::debug!(event = "fcp.web.api.action_accepted", "POST /api/action delivered");
            StatusCode::NO_CONTENT
        }
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            tracing::warn!(
                event = "fcp.web.api.action_channel_full",
                "POST /api/action rejected: channel full"
            );
            StatusCode::SERVICE_UNAVAILABLE
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
            tracing::warn!(
                event = "fcp.web.api.action_channel_closed",
                "POST /api/action rejected: session ended"
            );
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

pub async fn post_shutdown(State(state): State<WebAppState>) -> impl IntoResponse {
    tracing::info!(
        event = "fcp.web.api.shutdown",
        "POST /api/shutdown — graceful stop (same as Ctrl+C in the terminal)"
    );
    state.shutdown_token.cancel();
    StatusCode::NO_CONTENT
}
