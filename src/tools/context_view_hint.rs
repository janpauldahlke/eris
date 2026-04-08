//! Per-tool hints for [`crate::orchestrator::context::build_llm_view`]: each [`super::Tool`] may
//! override [`ToolContextViewHint::Default`] so the LLM-facing view stays lean without stringly `if`
//! chains in the orchestrator.

use serde::{Deserialize, Serialize};

/// Default cap for HTTP/API tools when using [`ToolContextViewHint::Snippet`].
pub const API_TOOL_SNIPPET_CHARS: usize = 320;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolContextViewHint {
    /// Use global `optimize_context_max_tool_snippet_chars` for the result body.
    Default,
    /// Keep the full stored tool line in the LLM view (still bounded at write time).
    Full,
    /// Marker only; no result body in the view.
    MarkerOnly,
    /// Truncate the result body to `max_chars` (Unicode-safe).
    Snippet { max_chars: usize },
}
