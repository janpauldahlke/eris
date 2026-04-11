//! System text injected around tool batches: success path (conversational `message_to_user`) and
//! failure recovery (honesty about errors). See [`crate::tools::specs::DESCRIPTOR_TOMLS`] for JIT descriptors.

/// Delimiters keep the line identifiable in logs and future condensation logic if needed.
pub const POST_TOOL_USER_REPLY_GUIDANCE: &str = r#"[FCP POST-TOOL — USER REPLY]
Your next JSON answer with status Idle must put the human-facing text in `message_to_user`: full sentences, plain language, and explain what the tool results mean for the user. Do not answer with raw JSON, one-line dumps, or robotic telegraphy. Use `thought` only for internal reasoning.
[/FCP POST-TOOL — USER REPLY]"#;

/// After `vault:read` (large file) or multi-chunk `web:fetch`: steer the model to pager / query tools without relying on the human typing tool names.
pub const POST_TOOL_STAGED_BUFFER_GUIDANCE: &str = r#"[FCP POST-TOOL — STAGED LARGE CONTENT]
The preceding tool output staged only part of a large body in ephemeral memory. You (the agent) must drive follow-up: if you need more text in order, call `ephemeral:buffer_page` with the `buffer_id` from the JSON receipt (`vault:read` → `buffer_id`; `web:fetch` → same short token in the receipt, often labeled `artifact_id`) and `page` 0, then 1, 2, … until covered. Default `page_size` is 1 (one stored chunk per page); raise `page_size` only when you want wider windows. Staging receipts include `chunk_navigation` (per-chunk byte offsets in the concatenation of chunks, head/tail previews, and a `first_heading` when a line starts with `#`). For keyword search inside the same buffer, call `ephemeral:buffer_query` with `buffer_id` set to that same token (legacy JSON key `artifact_id` is still accepted); responses include `query_note` (how terms were parsed), `match_count`, match-centered `snippet` text, optional `match_byte_offset_in_chunk`, and `top_k` may be up to 10. The server maps these short handles (e.g. `buf_1`) to storage keys; you do not need raw UUIDs. Do not ask the user to run these tools by name; continue in Reflect with tool_calls as needed.
For **vault** large files, the receipt includes `vault_lens` (file byte totals and `suggested_next_byte_offset` / `suggested_prev_byte_offset`). If the target section is **outside** the current lens, call `vault:read` again with the **same** `relative_path`, the **same** `buffer_id`, and `byte_offset` set to a suggested value to slide the lens **without** losing the handle—then use `buffer_query` / `buffer_page` inside the new window.
If the user later asks for a detailed or deep treatment of a named chapter, section, or topic, you must use `ephemeral:buffer_page` and/or `ephemeral:buffer_query` on this buffer until you have verified whether that text exists in the staged chunks. Do not write long invented “chapter content” from the table of contents or titles alone; if after paging/querying the buffer still has no such body text, say so plainly in `message_to_user`.
When you call those tools, copy `buffer_id` from the `[FCP_BUFFER_REF]` line, the receipt JSON, or the latest `[FCP BUFFER SESSION]` block **exactly**—same spelling and punctuation (e.g. `buf_2`). After each `ephemeral:buffer_page` result, read `navigation_hint` (especially when `page_count` is 1), `chunk_indices_in_page`, `remaining_chunk_indices`, and use JSON field `next_page` for the following call when it is not null.
[/FCP POST-TOOL — STAGED LARGE CONTENT]"#;

/// Appended to the tool-failure `[SYSTEM OVERRIDE: FUCKUP DETECTED]` recover line so Idle replies do not claim success.
pub const POST_TOOL_FAILURE_TRUST_GUIDANCE: &str = r#"[FCP TOOL FAILURE — USER REPLY]
A tool in the last batch failed. Your next JSON with status Idle must use `message_to_user` to state clearly that the operation did not complete, in plain language, using the error details above. Do not claim the tool succeeded, do not invent fetched or saved data, and do not imply Wikipedia/API/vault/memory worked unless a preceding system line explicitly says `Tool '...' succeeded` for that step. If some tools succeeded and another failed, say what worked and what failed.
[/FCP TOOL FAILURE — USER REPLY]"#;

/// System line for [`crate::orchestrator::r#loop::tool_batch::ToolBatchDecision::Recover`] after a recoverable tool execution failure.
pub fn recover_override_message_for_tool_failure(reason: &str) -> String {
    format!(
        "[SYSTEM OVERRIDE: FUCKUP DETECTED] Tool execution failed: {}\n\n{}",
        reason,
        POST_TOOL_FAILURE_TRUST_GUIDANCE
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_buffer_guidance_is_distinct_block() {
        assert!(POST_TOOL_STAGED_BUFFER_GUIDANCE.contains("ephemeral:buffer_page"));
        assert!(POST_TOOL_STAGED_BUFFER_GUIDANCE.contains("ephemeral:buffer_query"));
        assert!(POST_TOOL_STAGED_BUFFER_GUIDANCE.contains("artifact_id"));
        assert!(POST_TOOL_STAGED_BUFFER_GUIDANCE.contains("buffer_id"));
        assert!(POST_TOOL_STAGED_BUFFER_GUIDANCE.contains("FCP_BUFFER_REF"));
        assert!(POST_TOOL_STAGED_BUFFER_GUIDANCE.contains("buf_1"));
        assert!(POST_TOOL_STAGED_BUFFER_GUIDANCE.contains("next_page"));
        assert!(POST_TOOL_STAGED_BUFFER_GUIDANCE.contains("chunk_navigation"));
        assert!(POST_TOOL_STAGED_BUFFER_GUIDANCE.contains("navigation_hint"));
        assert!(POST_TOOL_STAGED_BUFFER_GUIDANCE.contains("query_note"));
    }

    #[test]
    fn recover_override_message_includes_failure_trust_guidance() {
        let msg = recover_override_message_for_tool_failure("network timeout");
        assert!(
            msg.contains(POST_TOOL_FAILURE_TRUST_GUIDANCE),
            "Recover message must embed POST_TOOL_FAILURE_TRUST_GUIDANCE so Idle cannot regress to false success claims"
        );
        assert!(msg.contains("network timeout"));
        assert!(msg.contains("[SYSTEM OVERRIDE: FUCKUP DETECTED]"));
    }
}
