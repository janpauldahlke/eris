//! Optional Discord sidecar (Serenity): same `UserAction` / presentation stream as web or TUI.

mod attachment;
mod format;
mod handler;
mod ready_signal;
mod sidecar;
mod typing_ctl;

pub use sidecar::run_discord_sidecar;
pub use typing_ctl::DiscordTypingCtl;

pub(crate) use ready_signal::DiscordReadySignal;
