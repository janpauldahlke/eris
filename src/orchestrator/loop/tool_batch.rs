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
    /// Duplicate-only batch: stay in Chat, disable tools for one reply (no Recover budget).
    SuppressOnlyIdlePass { message: String },
    /// Successful Chat batch on OpenRouter: omit tools for the next hop so the model
    /// must answer from the results (no dummy follow-up `memory:stage`). Local GBNF
    /// still returns [`Self::Continue`] so sequential tool hops in one turn stay legal.
    PostToolTalkPass { message: String },
    /// Abort turn on non-recoverable failure.
    Fatal(FcpError),
}
