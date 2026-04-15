//! Discord-only control plane: never crosses web or TUI presentation.

/// Commands the outbound sidecar to drive Discord’s channel typing indicator (HTTP only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscordTypingCtl {
    /// Start (or replace) a typing pulse until [`DiscordTypingCtl::StopPulse`] or drop.
    StartPulse,
    /// Stop the active typing pulse, if any.
    StopPulse,
}
