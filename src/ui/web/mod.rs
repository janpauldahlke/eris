//! Localhost web UI for chat (`eris chat --web`): Axum + SSE + minimal JS.

mod audio_handlers;
mod bridge;
mod console_handlers;
mod handlers;
mod router;
mod server;
mod settings_merge;
mod sse;
mod vision_handlers;

pub use server::{WebAppState, run_web_chat, run_web_chat_with_broadcast};
