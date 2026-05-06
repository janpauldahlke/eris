//! Recovery-related system line detection (must stay aligned with transitions / tool dispatch).

/// Prefix for JSON-parse and similar recoveries ([`crate::orchestrator::loop::directive_policy`]).
pub const FUCKUP_OVERRIDE_PREFIX: &str = "[SYSTEM OVERRIDE: FUCKUP DETECTED]";
/// Schema-targeted retry ([`crate::orchestrator::core::tool_dispatch`]).
pub const SYSTEM_RECOVERY_PREFIX: &str = "[SYSTEM RECOVERY]";
/// Duplicate tool batch suppression (starts with this exact phrase).
pub const DUPLICATE_TOOL_BATCH_PREFIX: &str = "[SYSTEM OVERRIDE] All requested tool calls";

pub fn is_recovery_system_content(content: &str) -> bool {
    let t = content.trim_start();
    t.starts_with(FUCKUP_OVERRIDE_PREFIX)
        || t.starts_with(SYSTEM_RECOVERY_PREFIX)
        || t.starts_with(DUPLICATE_TOOL_BATCH_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_fuckup_and_recovery_and_duplicate() {
        assert!(is_recovery_system_content(
            "[SYSTEM OVERRIDE: FUCKUP DETECTED] Invalid LLM Output: x"
        ));
        assert!(is_recovery_system_content(
            "[SYSTEM RECOVERY] Tool schema fault"
        ));
        assert!(is_recovery_system_content(
            "[SYSTEM OVERRIDE] All requested tool calls in this batch were suppressed"
        ));
    }

    #[test]
    fn rejects_jit_and_post_tool_and_plain() {
        assert!(!is_recovery_system_content(
            "[JIT TOOL GUIDANCE]\nfoo\n[/JIT TOOL GUIDANCE]"
        ));
        assert!(!is_recovery_system_content(
            "[FCP POST-TOOL — USER REPLY]\n...\n[/FCP POST-TOOL — USER REPLY]"
        ));
        assert!(!is_recovery_system_content("Tool 't' succeeded: ok"));
        assert!(!is_recovery_system_content(
            "[SYSTEM OVERRIDE] something else"
        ));
    }
}
