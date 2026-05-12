//! System text injected around tool batches: success path (conversational `message_to_user`) and
//! failure recovery (honesty about errors). See [`crate::tools::specs::DESCRIPTOR_TOMLS`] for JIT descriptors.

/// Delimiters keep the line identifiable in logs and future condensation logic if needed.
pub const POST_TOOL_USER_REPLY_GUIDANCE: &str = r#"[FCP POST-TOOL — USER REPLY]
Your next JSON answer with status Idle must put the human-facing text in `message_to_user`: full sentences, plain language, and explain what the tool results mean for the user. Do not answer with raw JSON, one-line dumps, or robotic telegraphy. Use `thought` only for internal reasoning.
[/FCP POST-TOOL — USER REPLY]"#;

/// Appended to the tool-failure protocol-fault recover line so Idle replies do not claim success.
pub const POST_TOOL_FAILURE_TRUST_GUIDANCE: &str = r#"[FCP TOOL FAILURE — USER REPLY]
A tool in the last batch failed. Your next JSON with status Idle must use `message_to_user` to state clearly that the operation did not complete, in plain language, using the error details above. Do not claim the tool succeeded, do not invent fetched or saved data, and do not imply Wikipedia/API/vault/memory worked unless a preceding system line explicitly says `Tool '...' succeeded` for that step. If some tools succeeded and another failed, say what worked and what failed.
[/FCP TOOL FAILURE — USER REPLY]"#;

/// System line for [`crate::orchestrator::r#loop::tool_batch::ToolBatchDecision::Recover`] after a recoverable tool execution failure.
pub fn recover_override_message_for_tool_failure(reason: &str) -> String {
    use crate::orchestrator::context::resolved_tool_recovery::PROTOCOL_FAULT_PREFIX;
    format!(
        "{PROTOCOL_FAULT_PREFIX}\n\nTool execution failed: {reason}\n\n{}",
        POST_TOOL_FAILURE_TRUST_GUIDANCE
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recover_override_message_includes_failure_trust_guidance() {
        let msg = recover_override_message_for_tool_failure("network timeout");
        assert!(
            msg.contains(POST_TOOL_FAILURE_TRUST_GUIDANCE),
            "Recover message must embed POST_TOOL_FAILURE_TRUST_GUIDANCE so Idle cannot regress to false success claims"
        );
        assert!(msg.contains("network timeout"));
        assert!(msg.contains(
            crate::orchestrator::context::resolved_tool_recovery::PROTOCOL_FAULT_PREFIX
        ));
    }
}
