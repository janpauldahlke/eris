//! Axum router for web chat.

use axum::routing::{get, post};
use axum::Router;

use super::handlers;
use super::sse;
use super::WebAppState;

pub fn web_chat_router(state: WebAppState) -> Router {
    Router::new()
        .route("/", get(handlers::chat_shell))
        .route("/assets/chat.js", get(handlers::chat_js))
        .route("/api/events", get(sse::session_events_sse))
        .route("/api/action", post(handlers::post_action))
        .route("/api/shutdown", post(handlers::post_shutdown))
        .with_state(state)
}
