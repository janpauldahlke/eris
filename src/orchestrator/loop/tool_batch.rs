use crate::executive::error::FcpError;

/// Outcome of a single tool-dispatch batch inside one orchestrator loop turn.
///
/// The coordinator consumes this enum and applies mutations through the
/// transition funnel, keeping policy/decision separate from state writes.
#[derive(Debug)]
pub enum ToolBatchDecision {
    /// Batch finished successfully; continue the loop.
    Continue,
    /// Stop the current turn and return control to idle.
    Halt,
    /// Retry with targeted schemas after parse/schema faults.
    RetryWithTargetedSchema { message: String },
    /// Enter recover state with a recoverable failure message.
    Recover { message: String },
    /// Abort turn on non-recoverable failure.
    Fatal(FcpError),
}
