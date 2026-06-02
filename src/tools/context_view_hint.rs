//! Per-tool hints for [`crate::orchestrator::context::build_llm_view`]: each [`super::Tool`] may
//! override [`ToolContextViewHint::Default`] so the LLM-facing view stays lean without stringly `if`
//! chains in the orchestrator.

use serde::{Deserialize, Serialize};

/// `web:artifact_query` returns chunk `matches` plus many `outbound_links`; the LLM view must stay
/// large enough that truncation does not hide `matches` (see orchestrator `build_llm_view`).
pub const ARTIFACT_QUERY_SNIPPET_CHARS: usize = 12_288;

/// Legacy cap for compact API tools (weather, mail check, etc.). Do not use for web/news receipts.
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
