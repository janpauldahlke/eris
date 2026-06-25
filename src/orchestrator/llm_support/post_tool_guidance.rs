//! System text injected around tool batches: success path (conversational `message_to_user`) and
//! failure recovery (honesty about errors). See [`crate::tools::specs::DESCRIPTOR_TOMLS`] for JIT descriptors.

/// Delimiters keep the line identifiable in logs and future condensation logic if needed.
pub const POST_TOOL_USER_REPLY_GUIDANCE: &str = r#"[FCP POST-TOOL — USER REPLY]
Your next JSON answer with status Idle must put the human-facing text in `message_to_user`: full sentences, plain language, and explain what the tool results mean for the user. Do not answer with raw JSON, one-line dumps, or robotic telegraphy. Use `thought` only for internal reasoning.
[/FCP POST-TOOL — USER REPLY]"#;

/// Injected instead of [`POST_TOOL_USER_REPLY_GUIDANCE`] when the model is in
/// Reflect state, so multi-step tool workflows (e.g. paginated doc:read for
/// full-book summarization) are not prematurely terminated.
pub const POST_TOOL_REFLECT_CONTINUATION_GUIDANCE: &str = r#"[FCP POST-TOOL — CONTINUE]
Tool results are above. You are in a multi-step workflow (Reflect). If you need more data to complete the user's request, continue with status Reflect and additional tool_calls. Only set status Idle with a complete answer in message_to_user when you have gathered enough information. Do not stop early with a progress report — the user asked for the full result.
[/FCP POST-TOOL — CONTINUE]"#;

/// Weather-only batches: LLM adds a brief human line; the runtime appends the deterministic report.
pub const POST_TOOL_WEATHER_COMMENT_GUIDANCE: &str = r#"[FCP POST-TOOL — WEATHER]
A pre-formatted markdown weather report is appended automatically after your reply — do not repeat temperatures, emoji forecast lines, or tables.
`message_to_user` must be one or two short sentences only (tone, heat/rain tip, or a direct answer). No bullet lists. Use `thought` for reasoning.
[/FCP POST-TOOL — WEATHER]"#;

/// Appended to the tool-failure protocol-fault recover line so Idle replies do not claim success.
pub const POST_TOOL_FAILURE_TRUST_GUIDANCE: &str = r#"[FCP TOOL FAILURE — USER REPLY]
A tool in the last batch failed. Your next JSON with status Idle must use `message_to_user` to state clearly that the operation did not complete, in plain language, using the error details above. Do not claim the tool succeeded, do not invent fetched or saved data, and do not imply Wikipedia/API/vault/memory worked unless a preceding system line explicitly says `Tool '...' succeeded` for that step. If some tools succeeded and another failed, say what worked and what failed.
[/FCP TOOL FAILURE — USER REPLY]"#;

/// System line for [`crate::orchestrator::r#loop::tool_batch::ToolBatchDecision::Recover`] after a recoverable tool execution failure.
pub fn recover_override_message_for_tool_failure(reason: &str) -> String {
    use crate::orchestrator::context::resolved_tool_recovery::PROTOCOL_FAULT_PREFIX;
    let mut out = format!(
        "{PROTOCOL_FAULT_PREFIX}\n\nTool execution failed: {reason}\n\n{}",
        POST_TOOL_FAILURE_TRUST_GUIDANCE
    );
    if let Some(extra) = web_find_before_refetch_recover_addon(reason) {
        out.push_str("\n\n");
        out.push_str(&extra);
    }
    out
}

/// Parse `artifact_id` from a `WEB_FIND_BEFORE_REFETCH` policy message.
pub fn parse_artifact_id_from_find_before_refetch_message(reason: &str) -> Option<String> {
    let needle = "artifact_id `";
    let start = reason.find(needle)? + needle.len();
    let rest = &reason[start..];
    let end = rest.find('`')?;
    let id = rest[..end].trim();
    if uuid::Uuid::parse_str(id).is_ok() {
        Some(id.to_string())
    } else {
        None
    }
}

/// Extra recover guidance when refetch was blocked pending `web:find`.
pub fn web_find_before_refetch_recover_addon(reason: &str) -> Option<String> {
    if !reason.contains(crate::tools::web::ledger::policy::WEB_FIND_BEFORE_REFETCH) {
        return None;
    }
    let mut block = String::from(
        "[FCP WEB — USE web:find]\n\
         web:find is available in this recovery pass (alongside web:fetch). \
         Do not claim web:find is missing from your toolset.\n",
    );
    if let Some(aid) = parse_artifact_id_from_find_before_refetch_message(reason) {
        block.push_str(&format!(
            "Call web:find with artifact_id \"{aid}\" and a query matching what you need \
             (e.g. transmission routes) before any web:fetch on the same host.\n"
        ));
    } else {
        block.push_str(
            "Call web:find on the artifact_id from the error above before any web:fetch on that host.\n",
        );
    }
    block.push_str("[/FCP WEB — USE web:find]");
    Some(block)
}

/// When `web:fetch` or `web:search` is targeted for Recover/GBNF, always include `web:find` if allowed.
pub fn ensure_web_find_paired_with_fetch_tools(
    targeted_tools: &mut std::collections::HashSet<String>,
    allowed: &std::collections::HashSet<String>,
) {
    let needs_find = targeted_tools.contains("web:fetch") || targeted_tools.contains("web:search");
    if needs_find && allowed.contains("web:find") {
        targeted_tools.insert("web:find".to_string());
    }
}

/// System line when every tool intent in a batch was duplicate-suppressed (no Recover hop).
pub const DUPLICATE_SUPPRESS_IDLE_GUIDANCE: &str = r#"[FCP DUPLICATE TOOL — USER REPLY]
All tool_calls in your last batch were skipped as duplicates of calls already made this turn. Do not repeat them. Reply with status Idle, a non-empty message_to_user summarizing prior tool results, and tool_calls [].
[/FCP DUPLICATE TOOL — USER REPLY]"#;

/// True when the latest user turn asks to remember, save, or catalog an uploaded image.
pub fn user_wants_media_catalog(user: &str) -> bool {
    let lower = user.to_lowercase();
    let image_hint = lower.contains("image")
        || lower.contains("photo")
        || lower.contains("picture")
        || lower.contains("pic")
        || lower.contains("upload")
        || lower.contains("attached")
        || lower.contains("this");
    lower.contains("remember this")
        || lower.contains("save this")
        || lower.contains("catalog this")
        || lower.contains("keep in media")
        || lower.contains("keep this image")
        || lower.contains("keep this photo")
        || (lower.contains("remember") && image_hint)
        || (lower.contains("save") && image_hint)
        || (lower.contains("catalog") && image_hint)
}

/// System line after successful `vision:see` when the user asked to remember the image.
pub fn vision_see_catalog_nudge(relative_path: &str, description: &str) -> String {
    format!(
        r#"[FCP MEDIA — CATALOG NEXT]
vision:see succeeded for `{relative_path}`. The user asked to remember this image.
Your next tool batch must call media:catalog with:
- relative_path: "{relative_path}"
- description: use the vision description below (trim if very long)
Description from vision:see:
{description}
Invent a short title from what you saw; do not use the user's command phrase as title.
[/FCP MEDIA — CATALOG NEXT]"#
    )
}

/// After any `web:*` tool is armed for protocol Recover, offer the full browser stack when allowed.
pub fn expand_web_tools_for_protocol_recover(
    targeted_tools: &mut std::collections::HashSet<String>,
    allowed: &std::collections::HashSet<String>,
) {
    if !targeted_tools.iter().any(|n| n.starts_with("web:")) {
        return;
    }
    for name in ["web:fetch", "web:find", "web:search"] {
        if allowed.contains(name) {
            targeted_tools.insert(name.to_string());
        }
    }
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

    #[test]
    fn user_wants_media_catalog_detects_remember_this() {
        assert!(user_wants_media_catalog("remember this"));
        assert!(user_wants_media_catalog("Please remember this photo"));
        assert!(!user_wants_media_catalog("what is in this image?"));
    }

    #[test]
    fn vision_see_catalog_nudge_includes_path_and_description() {
        let msg = vision_see_catalog_nudge(
            "99_USER_UPLOADED/images/abc.jpg",
            "A red truck.",
        );
        assert!(msg.contains("media:catalog"));
        assert!(msg.contains("99_USER_UPLOADED/images/abc.jpg"));
        assert!(msg.contains("A red truck."));
    }

    #[test]
    fn recover_web_find_before_refetch_includes_find_guidance() {
        let reason = "Policy violation [WEB_FIND_BEFORE_REFETCH]: web:fetch blocked for host `bbc.com`: try web:find on artifact_id `8ff6f8c6-422d-49ec-b1c8-1fb260dc9bc9` before fetching again";
        let msg = recover_override_message_for_tool_failure(reason);
        assert!(msg.contains("[FCP WEB — USE web:find]"));
        assert!(msg.contains("8ff6f8c6-422d-49ec-b1c8-1fb260dc9bc9"));
        assert!(msg.contains("web:find is available"));
    }

    #[test]
    fn expand_web_tools_adds_fetch_find_search() {
        let allowed: std::collections::HashSet<String> = [
            "web:fetch".to_string(),
            "web:find".to_string(),
            "web:search".to_string(),
        ]
        .into_iter()
        .collect();
        let mut targeted = std::collections::HashSet::from(["web:find".to_string()]);
        expand_web_tools_for_protocol_recover(&mut targeted, &allowed);
        assert!(targeted.contains("web:fetch"));
        assert!(targeted.contains("web:search"));
    }

    #[test]
    fn ensure_web_find_paired_when_fetch_targeted() {
        use std::collections::HashSet;
        let allowed: HashSet<String> =
            ["web:fetch", "web:find"].iter().map(|s| (*s).to_string()).collect();
        let mut targeted: HashSet<String> = ["web:fetch"].iter().map(|s| (*s).to_string()).collect();
        super::ensure_web_find_paired_with_fetch_tools(&mut targeted, &allowed);
        assert!(targeted.contains("web:find"));
    }
}
