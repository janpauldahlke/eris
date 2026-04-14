//! First-run dependency and vault-path welder (stdin TTY, before ratatui raw mode).
//!
//! Skipped when `ERIS_SKIP_SETUP=1`, `CI=true`, or stdin is not a terminal.

mod gate;
mod hint;
mod prompts;
mod report;
mod run;

pub use hint::IgnitionWorkspaceHint;
pub use report::WelderReport;
pub use run::run_welder_before_chat;
