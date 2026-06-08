//! Axum router for web chat.

use axum::Router;
use axum::routing::{get, post};

use super::WebAppState;
use super::handlers;
use super::sse;
use super::vision_handlers;
use super::audio_handlers;

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
        .route("/api/audio/status", get(audio_handlers::audio_status))
        .route("/api/audio/upload", post(audio_handlers::post_audio_upload))
        .route(
            "/api/audio/preview/{filename}",
            get(audio_handlers::get_audio_preview),
        )
        .with_state(state)
}
