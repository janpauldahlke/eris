//! When the interactive welder is allowed to run.

/// `false` when `ERIS_SKIP_SETUP=1`.
pub(super) fn welder_enabled_by_env() -> bool {
    std::env::var("ERIS_SKIP_SETUP").ok().as_deref() != Some("1")
}

pub(super) fn stdin_is_interactive() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stdin())
}
