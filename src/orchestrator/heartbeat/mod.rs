//! Idle timeout: watch channel fires when user input has been idle too long.

mod monitor;

pub use monitor::spawn_heartbeat_monitor;
