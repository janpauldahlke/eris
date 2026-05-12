//! Recovery-related system line detection (must stay aligned with transitions / tool dispatch).

/// Prefix for JSON-parse and protocol violations ([`crate::orchestrator::loop::directive_policy`], tool failures).
pub const PROTOCOL_FAULT_PREFIX: &str = "[SYSTEM] Invalid model output";
/// One-line UI telemetry when the model-facing message includes JSON repair markers.
pub const JSON_REPAIR_TELEMETRY: &str = "[SYSTEM] JSON repair";
/// Schema-targeted retry ([`crate::orchestrator::core::tool_dispatch`]).
pub const SYSTEM_RECOVERY_PREFIX: &str = "[SYSTEM] Recovery";
/// Duplicate tool batch suppression (starts with this exact phrase).
pub const DUPLICATE_TOOL_BATCH_PREFIX: &str = "[SYSTEM] Tool batch suppressed";

pub fn is_recovery_system_content(content: &str) -> bool {
    let t = content.trim_start();
    t.starts_with(PROTOCOL_FAULT_PREFIX)
        || t.starts_with(JSON_REPAIR_TELEMETRY)
        || t.starts_with(SYSTEM_RECOVERY_PREFIX)
        || t.starts_with(DUPLICATE_TOOL_BATCH_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_protocol_fault_recovery_and_duplicate() {
        assert!(is_recovery_system_content(
            "[SYSTEM] Invalid model output: trailing comma"
        ));
        assert!(is_recovery_system_content(
            "[SYSTEM] JSON repair"
        ));
        assert!(is_recovery_system_content(
            "[SYSTEM] Recovery — schema retry"
        ));
        assert!(is_recovery_system_content(
            "[SYSTEM] Tool batch suppressed — duplicates"
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
            "[SYSTEM] something else without the batch prefix"
        ));
    }
}
