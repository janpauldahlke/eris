//! Ratatui + crossterm terminal UI. Core code uses [`crate::presentation`] only.

mod app;
mod render;
mod setup;

pub use app::TuiApp;
pub use setup::{restore_terminal, setup_terminal};
