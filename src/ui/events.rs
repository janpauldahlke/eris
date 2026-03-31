use crossterm::event::KeyEvent;
use crate::orchestrator::state::AgentState;

pub enum TuiEvent {
    Tick,
    Input(KeyEvent),
    StateUpdate(AgentStateUpdate),
    IncomingMessage(String), // Stream of assistant messages
    SystemError(String),     // System Errors / Telemetry
}

#[derive(Clone)]
pub struct AgentStateUpdate {
    pub state: AgentState,
    pub tool_rounds: u8,
    pub recovery_count: u8,
    pub active_task: Option<String>,
}
