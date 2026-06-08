//! Axum router for web chat.

use axum::Router;
use axum::routing::{get, post};

use super::WebAppState;
use super::handlers;
use super::sse;
use super::vision_handlers;

pub fn web_chat_router(state: WebAppState) -> Router {
    Router::new()
        .route("/", get(handlers::chat_shell))
        .route("/assets/chat.js", get(handlers::chat_js))
        .route("/api/events", get(sse::session_events_sse))
        .route("/api/action", post(handlers::post_action))
        .route("/api/shutdown", post(handlers::post_shutdown))
        .route("/api/vision/status", get(vision_handlers::vision_status))
        .route("/api/vision/upload", post(vision_handlers::post_vision_upload))
        .route(
            "/api/vision/preview/{filename}",
            get(vision_handlers::get_vision_preview),
        )
        .with_state(state)
}
