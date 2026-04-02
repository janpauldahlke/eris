//! Stable `issue` codes for pre-LLM routing and related orchestrator logs.
//! Renames are breaking changes for operators grepping logs; document in release notes.

/// Use on every routing decision event so plain-text logs remain filterable when
/// `tracing_subscriber` uses `with_target(false)` (targets are not printed).
pub const CATEGORY_ROUTING: &str = "routing";

pub const OUTCOME_CONVERSATIONAL: &str = "conversational";
pub const OUTCOME_TOOL_FALLBACK: &str = "tool_fallback";
pub const OUTCOME_TOOL_MATCHED: &str = "tool_matched";

pub const ISSUE_STEP_EMPTY_USER_SY_FNORD: &str = "STEP_EMPTY_USER_SY_FNORD";
pub const ISSUE_PRELLM_CONV_ALARM: &str = "PRELLM_CONV_ALARM";
pub const ISSUE_PRELLM_CONV_SHORT_INPUT: &str = "PRELLM_CONV_SHORT_INPUT";
pub const ISSUE_PRELLM_ROUTER_UNAVAILABLE: &str = "PRELLM_ROUTER_UNAVAILABLE";
pub const ISSUE_PRELLM_SEMANTIC_EMPTY: &str = "PRELLM_SEMANTIC_EMPTY";
pub const ISSUE_PRELLM_SEMANTIC_HIT: &str = "PRELLM_SEMANTIC_HIT";
pub const ISSUE_PRELLM_MATCH_ERROR: &str = "PRELLM_MATCH_ERROR";

/// Maps the physical `(tools_needed, router_match_len)` after routing completes.
/// Does not distinguish alarm vs short-input (both `tools_needed == false`, `len == 0`); use `issue` for that.
#[must_use]
pub fn outcome_from_pre_llm_tuple(tools_needed: bool, router_match_len: usize) -> &'static str {
    if !tools_needed {
        OUTCOME_CONVERSATIONAL
    } else if router_match_len == 0 {
        OUTCOME_TOOL_FALLBACK
    } else {
        OUTCOME_TOOL_MATCHED
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_from_pre_llm_tuple_maps_physical_state() {
        assert_eq!(outcome_from_pre_llm_tuple(false, 0), OUTCOME_CONVERSATIONAL);
        assert_eq!(outcome_from_pre_llm_tuple(false, 99), OUTCOME_CONVERSATIONAL);
        assert_eq!(outcome_from_pre_llm_tuple(true, 0), OUTCOME_TOOL_FALLBACK);
        assert_eq!(outcome_from_pre_llm_tuple(true, 1), OUTCOME_TOOL_MATCHED);
        assert_eq!(outcome_from_pre_llm_tuple(true, 5), OUTCOME_TOOL_MATCHED);
    }
}
