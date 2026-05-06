//! System prompt assembly ([`ContextAssembler`]), LLM-only chat projection ([`build_llm_view`]),
//! sliding-window condensation, and slim tool phrase compendium.

mod assembler;
mod compendium;
pub mod resolved_tool_recovery;
pub mod stack_lines;
mod view;
mod window;

pub use assembler::ContextAssembler;
pub use compendium::{build_phrase_compendium, typical_phrasing_for_tool};
pub use stack_lines::{
    ParsedSystemLine, TOOL_SUCCESS_INFIX, TOOL_SUCCESS_PREFIX, ToolSuccessLine,
    format_tool_success_line, parse_system_line, try_parse_tool_success_line,
};
pub use view::{
    ContextViewSettings, FCP_TOOL_DEFS_BEGIN, FCP_TOOL_DEFS_END, SlimToolDefsMeta, build_llm_view,
    slim_tool_definitions_inner,
};
pub use window::{
    CondensationPlan, ROLLING_SUMMARY_KIND, ROLLING_SUMMARY_TITLE, RollingSummaryV1, StackHead,
    build_summarization_stack, condensation_system_instruction, estimate_message_tokens,
    estimate_stack_tokens, is_jit_system_message, is_rolling_summary_message,
    normalize_rolling_summary_response, plan_sliding_condensation, retain_budget_tokens,
    rolling_summary_system_message, split_stack_head, split_tail_fold_and_keep, tail_after_head,
};
