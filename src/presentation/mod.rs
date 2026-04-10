//! Presentation-neutral types shared by CLI (Ratatui) and future web UI.
//! Interactive chat requires a live `presentation_tx`; `None` is for headless tests and batch runners.

use serde::Serialize;

use crate::orchestrator::state::AgentState;

/// Prefix applied by the orchestrator when turning [`UserAction::SystemInject`] into a `user` line.
pub const SYSTEM_ALARM_PREFIX: &str = "[SYSTEM OVERRIDE - ALARM TRIGGERED]: ";

/// Alarm notification from the scheduler: plain timer/wall, or agenda-linked (needs confirmation flow).
#[derive(Debug, Clone, Serialize)]
pub enum AlarmPayload {
    Plain(String),
    AgendaLinked {
        agenda_task_id: String,
        label: String,
        alarm_record_id: String,
        /// Seconds after scheduled fire (e.g. app was offline).
        seconds_late: u64,
    },
}

#[derive(Debug, Clone, Serialize)]
pub enum UserAction {
    Submit(String),
    CancelCurrentTurn,
    /// Asynchronous clock/alarm injected via the active view; raw label only (prefix added in orchestrator).
    SystemInject(String),
    /// Agenda-linked alarm: orchestrator injects confirmation framing (same turn as live alarm).
    AgendaAlarmPending {
        agenda_task_id: String,
        label: String,
        alarm_record_id: String,
        seconds_late: u64,
    },
}

/// Outbound updates from core to the active presentation (terminal or web).
#[derive(Debug, Clone, Serialize)]
pub enum SessionEvent {
    StateUpdate(AgentStateUpdate),
    IncomingMessage(String),
    SystemError(String),
    /// Fired by the alarm scheduler; the active view forwards to [`UserAction`] (plain inject or agenda confirmation).
    SystemAlarm(AlarmPayload),
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentStateUpdate {
    pub state: AgentState,
    pub tool_rounds: u8,
    /// Configured per-turn cap (shown in Status as `T:…/max`).
    pub max_tool_rounds: u8,
    pub recovery_count: u8,
    /// Configured recovery budget (shown in Status as `R:…/max`).
    pub max_recovery_attempts: u8,
    pub active_task: Option<String>,
    /// Status-only hint while tools run (e.g. `Tools: …`); user text stays on the main transcript.
    pub activity_line: Option<String>,
    pub queued_inputs: usize,
    pub router_ms: u64,
    pub llm_ms: u64,
    pub tool_ms: u64,
    pub total_ms: u64,
    pub top_tool_match: Option<String>,
}
