use crossterm::event::KeyEvent;
use crate::orchestrator::state::AgentState;

/// Prefix applied by the orchestrator when turning [`UserAction::SystemInject`] into a `user` line.
pub const SYSTEM_ALARM_PREFIX: &str = "[SYSTEM OVERRIDE - ALARM TRIGGERED]: ";

#[derive(Debug, Clone)]
pub enum UserAction {
    Submit(String),
    CancelCurrentTurn,
    /// Asynchronous clock/alarm injected via TUI relay; raw label only (prefix added in orchestrator).
    SystemInject(String),
}

pub enum TuiEvent {
    Tick,
    Input(KeyEvent),
    StateUpdate(AgentStateUpdate),
    IncomingMessage(String),
    SystemError(String),     // System Errors / Telemetry
    /// Fired by the alarm scheduler; TUI must only forward to [`UserAction::SystemInject`].
    SystemAlarm(String),
}

#[derive(Clone)]
pub struct AgentStateUpdate {
    pub state: AgentState,
    pub tool_rounds: u8,
    pub recovery_count: u8,
    pub active_task: Option<String>,
    /// Short hint shown in Status while tools run (not on the main transcript).
    pub activity_line: Option<String>,
    pub queued_inputs: usize,
    pub router_ms: u64,
    pub llm_ms: u64,
    pub tool_ms: u64,
    pub total_ms: u64,
    pub top_tool_match: Option<String>,
}
