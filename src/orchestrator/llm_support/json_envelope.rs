//! Split a raw LLM reply into the leading JSON object and any trailing text.

use crate::orchestrator::state::LlmResponse;

/// First line of the JSON-parse recovery hint; also used to detect this path when shortening UI copy.
pub const FCP_JSON_REPAIR_MARKER: &str = "[FCP JSON REPAIR]";

/// One-line [`SessionEvent::SystemError`] when the model-facing message includes [`FCP_JSON_REPAIR_MARKER`].
pub const JSON_REPAIR_UI_SUMMARY: &str = "[SYSTEM OVERRIDE: FUCKUP DETECTED] JSON repair";

/// Human- and model-oriented hints appended after serde’s error when [`LlmResponse`] parsing fails.
/// Serde’s `expected ',' or '}'` near a `]` is often misread as a comma problem; this steers toward
/// the real issue (extra/missing `}` around `tool_calls` entries).
const LLM_JSON_PARSE_RECOVERY_HINT_BODY: &str = r##"The reply you just generated for this turn was not valid protocol JSON (single object: thought, status, message_to_user, tool_calls).

The serde error often means unbalanced { } braces — not “add a comma”. When `tool_calls` has exactly ONE item, a common mistake is: after the `}` that closes `args`, you must emit one more `}` to close the tool object before `]` ends the array.

Invalid: "tool_calls":[ {"name":"t","args":{...} ]
Valid:   "tool_calls":[ {"name":"t","args":{...} } ]

If the error says expected ',' or '}' at a line that shows `]`, you probably need that extra `}`.

Reply with one JSON object only (thought, status, message_to_user, tool_calls). No prose or markdown outside it."##;

/// Max chars for the optional single-line preview appended to JSON-parse recovery messages.
pub const LLM_JSON_PARSE_RECOVERY_PREVIEW_MAX_CHARS: usize = 200;

/// Collapses newlines to spaces and caps length for a safe one-line model-facing preview.
pub fn capped_single_line_protocol_preview(raw: &str, max_chars: usize) -> String {
    let collapsed: String = raw
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    let collapsed = collapsed.trim();
    let count = collapsed.chars().count();
    if count <= max_chars {
        return collapsed.to_string();
    }
    let take = max_chars.saturating_sub(1);
    let mut s: String = collapsed.chars().take(take).collect();
    s.push('…');
    s
}

/// Parse the leading JSON object as [`LlmResponse`] after tool-call normalization (same contract as the orchestrator directive path).
pub fn parse_llm_response_protocol(raw: &str) -> Result<LlmResponse, serde_json::Error> {
    let json_str = split_leading_json_object(raw).0;
    let mut parsed: LlmResponse = serde_json::from_str(json_str)?;
    parsed.normalize_tool_calls();
    Ok(parsed)
}

/// Full recovery payload for [`crate::orchestrator::state::LoopDirective::RecoverFromFuckup`].
pub fn llm_json_parse_recovery_message(err: &serde_json::Error) -> String {
    format!(
        "{err}\n\n{}\n{}",
        FCP_JSON_REPAIR_MARKER, LLM_JSON_PARSE_RECOVERY_HINT_BODY
    )
}

/// Same as [`llm_json_parse_recovery_message`] plus a capped single-line excerpt of the raw model output (for the recovery LLM pass only).
pub fn llm_json_parse_recovery_message_with_excerpt(err: &serde_json::Error, raw: &str) -> String {
    let preview =
        capped_single_line_protocol_preview(raw, LLM_JSON_PARSE_RECOVERY_PREVIEW_MAX_CHARS);
    format!(
        "{err}\n\n{}\n{}\n\n[FCP: protocol_preview]\n{}",
        FCP_JSON_REPAIR_MARKER, LLM_JSON_PARSE_RECOVERY_HINT_BODY, preview
    )
}

/// Returns `(json_object, remainder)` where `json_object` spans from the first `{` through its
/// matching closing `}`, respecting JSON string escapes. `remainder` is everything after that `}`.
///
/// If no balanced object is found from the first `{`, falls back to the slice from first `{` to
/// last `}` and the bytes after that closing brace (legacy behavior).
pub fn split_leading_json_object(raw: &str) -> (&str, &str) {
    let Some(start) = raw.find('{') else {
        return (raw, "");
    };
    let bytes = raw.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for i in start..bytes.len() {
        let b = bytes[i];
        if in_string {
            if escape {
                escape = false;
                continue;
            }
            if b == b'\\' {
                escape = true;
                continue;
            }
            if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return (&raw[start..=i], &raw[i + 1..]);
                }
            }
            _ => {}
        }
    }
    if let Some(end) = raw.rfind('}') {
        if end >= start {
            return (&raw[start..=end], &raw[end + 1..]);
        }
    }
    (raw, "")
}

/// `true` when there is non-whitespace after the first complete JSON object **and** that object
/// parses as [`LlmResponse`] (so we do not treat malformed all-in-one blobs as this violation).
pub fn trailing_content_after_valid_llm_json(raw: &str) -> bool {
    let (json_str, tail) = split_leading_json_object(raw);
    if tail.trim().is_empty() {
        return false;
    }
    serde_json::from_str::<LlmResponse>(json_str).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_balanced_then_markdown() {
        let raw = "{\"a\":1,\"b\":\"x}\"}\n## Poem\nline";
        let (j, tail) = split_leading_json_object(raw);
        assert_eq!(j, "{\"a\":1,\"b\":\"x}\"}");
        assert_eq!(tail, "\n## Poem\nline");
    }

    #[test]
    fn split_no_trailing() {
        let raw = r#"{"thought":"","status":"Idle","message_to_user":"hi","tool_calls":[]}"#;
        let (j, tail) = split_leading_json_object(raw);
        assert_eq!(tail, "");
        assert!(j.contains("message_to_user"));
    }

    #[test]
    fn violation_when_valid_json_then_prose() {
        let raw = r#"{"thought":"t","status":"Idle","message_to_user":"intro","tool_calls":[]}

# Extra"#;
        assert!(trailing_content_after_valid_llm_json(raw));
    }

    #[test]
    fn no_violation_when_no_json_brace() {
        assert!(!trailing_content_after_valid_llm_json("plain text only"));
    }

    #[test]
    fn json_parse_recovery_message_includes_brace_and_tool_calls_hints() {
        let err = serde_json::from_str::<LlmResponse>("not json").expect_err("invalid json");
        let msg = llm_json_parse_recovery_message(&err);
        assert!(msg.contains("tool_calls"));
        assert!(msg.contains(FCP_JSON_REPAIR_MARKER));
        assert!(msg.contains("one more"));
    }

    #[test]
    fn json_parse_recovery_with_excerpt_includes_preview_marker() {
        let raw = "That's prose\nnot json";
        let err = parse_llm_response_protocol(raw).expect_err("expected parse failure");
        let msg = llm_json_parse_recovery_message_with_excerpt(&err, raw);
        assert!(msg.contains("[FCP: protocol_preview]"));
        assert!(msg.contains("That's prose"));
    }
}
