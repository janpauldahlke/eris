//! Localhost web UI for chat (`eris chat --web`): Axum + SSE + minimal JS.

mod bridge;
mod handlers;
mod router;
mod server;
mod sse;

pub use server::{run_web_chat, run_web_chat_with_broadcast, WebAppState};
