use crossterm::event::KeyEvent;
use crate::orchestrator::state::AgentState;

#[derive(Debug, Clone)]
pub enum UserAction {
    Submit(String),
    CancelCurrentTurn,
}

pub enum TuiEvent {
    Tick,
    Input(KeyEvent),
    StateUpdate(AgentStateUpdate),
    IncomingMessage(String),
    SystemError(String),     // System Errors / Telemetry
}

#[derive(Clone)]
pub struct AgentStateUpdate {
    pub state: AgentState,
    pub tool_rounds: u8,
    pub recovery_count: u8,
    pub active_task: Option<String>,
    pub queued_inputs: usize,
    pub router_ms: u64,
    pub llm_ms: u64,
    pub tool_ms: u64,
    pub total_ms: u64,
    pub top_tool_match: Option<String>,
}
