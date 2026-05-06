//! Axum router for web chat.

use axum::Router;
use axum::routing::{get, post};

use super::WebAppState;
use super::handlers;
use super::sse;

pub fn web_chat_router(state: WebAppState) -> Router {
    Router::new()
        .route("/", get(handlers::chat_shell))
        .route("/assets/chat.js", get(handlers::chat_js))
        .route("/api/events", get(sse::session_events_sse))
        .route("/api/action", post(handlers::post_action))
        .route("/api/shutdown", post(handlers::post_shutdown))
        .with_state(state)
}
