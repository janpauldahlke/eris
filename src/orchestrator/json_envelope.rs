//! Split a raw LLM reply into the leading JSON object and any trailing text.

use crate::orchestrator::state::LlmResponse;

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
}
