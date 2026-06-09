//! Axum router for web chat.

use axum::Router;
use axum::routing::{get, post};

use super::WebAppState;
use super::audio_handlers;
use super::console_handlers;
use super::handlers;
use super::sse;
use super::vision_handlers;

pub fn web_chat_router(state: WebAppState) -> Router {
    Router::new()
        .route("/", get(handlers::chat_shell))
        .route("/assets/chat.js", get(handlers::chat_js))
        .route("/assets/console.js", get(console_handlers::console_js))
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
        .route("/api/console/identity", get(console_handlers::get_identity).put(console_handlers::put_identity))
        .route(
            "/api/console/settings",
            get(console_handlers::get_settings).put(console_handlers::put_settings),
        )
        .route(
            "/api/console/tools",
            get(console_handlers::get_tools).put(console_handlers::put_tools),
        )
        .route("/api/console/skills", get(console_handlers::get_skills))
        .route(
            "/api/console/skills/{name}",
            get(console_handlers::get_skill_detail),
        )
        .route("/api/console/memory", get(console_handlers::get_memory))
        .route("/api/console/memory/note", get(console_handlers::get_memory_note))
        .route("/api/console/uploads", get(console_handlers::get_uploads))
        .route(
            "/api/console/uploads/files",
            post(console_handlers::post_upload_file),
        )
        .with_state(state)
}
