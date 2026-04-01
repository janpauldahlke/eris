use crate::executive::error::FcpError;
use crate::orchestrator::state::ToolCall;

/// Canonical transition contract for the coordinator state machine.
///
/// Policy modules output `StateTransition`; `core.rs` is the only place that
/// mutates orchestrator state in response.
#[derive(Debug)]
pub enum StateTransition {
    ExecuteTools(Vec<ToolCall>),
    Halt,
    Recover { message: String, schema_retry: bool },
    ShiftToReflection,
    Fatal(FcpError),
    Continue,
}

/// Control signal returned by `apply_transition`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionControl {
    ContinueLoop,
    ReturnOk,
}
