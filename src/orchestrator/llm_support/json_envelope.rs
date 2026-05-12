//! Split a raw LLM reply into the leading JSON object and any trailing text.

use crate::orchestrator::state::LlmResponse;
use schemars::schema::{
    InstanceType, RootSchema, Schema, SchemaObject, SingleOrVec,
};
use std::borrow::Cow;

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

fn schema_object_metadata_description(obj: &SchemaObject) -> Option<String> {
    obj.metadata
        .as_ref()
        .and_then(|m| m.description.as_ref())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn schema_object_is_non_trivially_complex(obj: &SchemaObject) -> bool {
    obj.subschemas.as_ref().is_some_and(|sub| {
        sub.one_of.as_ref().is_some_and(|v| !v.is_empty())
            || sub.any_of.as_ref().is_some_and(|v| !v.is_empty())
            || sub.not.is_some()
            || sub.if_schema.is_some()
    })
}

fn resolve_schema<'a>(root: &'a RootSchema, s: &'a Schema) -> Cow<'a, Schema> {
    if let Schema::Object(o) = s {
        if let Some(r) = o.reference.as_deref() {
            if let Some(def_name) = r.strip_prefix("#/definitions/")
                && let Some(def) = root.definitions.get(def_name)
            {
                return Cow::Borrowed(def);
            }
        }
    }
    Cow::Borrowed(s)
}

fn format_instance_types(it: &Option<SingleOrVec<InstanceType>>) -> Option<String> {
    let it = it.as_ref()?;
    let parts: Vec<&'static str> = match it {
        SingleOrVec::Single(t) => vec![instance_type_label(t.as_ref())],
        SingleOrVec::Vec(v) => v.iter().map(|t| instance_type_label(t)).collect(),
    };
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" | "))
    }
}

fn instance_type_label(t: &InstanceType) -> &'static str {
    match t {
        InstanceType::Null => "null",
        InstanceType::Boolean => "boolean",
        InstanceType::Object => "object",
        InstanceType::Array => "array",
        InstanceType::Number => "number",
        InstanceType::String => "string",
        InstanceType::Integer => "integer",
    }
}

fn value_to_inline_enum(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(format!("{s:?}")),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Null => Some("null".to_string()),
        _ => None,
    }
}

fn format_enum_inline(values: &[serde_json::Value]) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for v in values {
        if let Some(p) = value_to_inline_enum(v) {
            parts.push(p);
        } else {
            return None;
        }
    }
    if parts.is_empty() {
        None
    } else if parts.len() == 1 {
        Some(parts[0].clone())
    } else {
        let last = parts.pop()?;
        Some(format!("{} or {}", parts.join(", "), last))
    }
}

fn schema_to_json_fallback(s: &Schema) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "<schema>".to_string())
}

/// Formats a property schema for natural-language recovery (shallow; nested object one level).
fn format_property_detail(
    root: &RootSchema,
    _prop_name: &str,
    prop_schema: &Schema,
    depth: u8,
) -> String {
    let resolved = resolve_schema(root, prop_schema);
    let s = resolved.as_ref();
    match s {
        Schema::Bool(true) => "any".to_string(),
        Schema::Bool(false) => "never".to_string(),
        Schema::Object(o) => {
            if schema_object_is_non_trivially_complex(o) {
                return schema_to_json_fallback(s);
            }
            if let Some(ot) = &o.object {
                if depth == 0 && !ot.properties.is_empty() {
                    let mut nested_lines: Vec<String> = Vec::new();
                    let mut nk: Vec<&String> = ot.properties.keys().collect();
                    nk.sort();
                    for k in nk {
                        let Some(sub) = ot.properties.get(k) else {
                            continue;
                        };
                        let detail = format_property_detail(root, k, sub, depth.saturating_add(1));
                        nested_lines.push(format!("  - {k}: {detail}"));
                    }
                    return format!("object:\n{}", nested_lines.join("\n"));
                }
            }
            let mut bits: Vec<String> = Vec::new();
            if let Some(t) = format_instance_types(&o.instance_type) {
                bits.push(t);
            }
            if let Some(en) = o.enum_values.as_ref()
                && let Some(inline) = format_enum_inline(en)
            {
                bits.push(inline);
            }
            if bits.is_empty() {
                schema_to_json_fallback(s)
            } else {
                bits.join(", ")
            }
        }
    }
}

/// Build a natural-language description of a tool's expected arguments.
/// Used for grammar-path schema recovery (instead of raw JSON Schema).
pub fn natural_language_schema_description(
    tool_name: &str,
    schema: &RootSchema,
    error_message: &str,
) -> String {
    let err_line = error_message.trim();
    let obj = schema.schema.object.as_deref();
    let Some(ov) = obj else {
        let fallback = serde_json::to_string_pretty(schema)
            .unwrap_or_else(|_| "<unserializable schema>".to_string());
        return format!(
            "Tool \"{tool_name}\" rejected your arguments.\n\nError: {err_line}\n\nExpected arguments (raw JSON Schema — root was not an object schema):\n{fallback}\n\nRetry with corrected tool_calls."
        );
    };
    if ov.properties.is_empty() {
        return format!(
            "Tool \"{tool_name}\" rejected your arguments.\n\nError: {err_line}\n\nExpected arguments:\nNo arguments required.\n\nRetry with corrected tool_calls."
        );
    }
    let mut lines: Vec<String> = Vec::new();
    let mut keys: Vec<&String> = ov.properties.keys().collect();
    keys.sort();
    for key in keys {
        let Some(raw_prop) = ov.properties.get(key.as_str()) else {
            continue;
        };
        let prop_schema = resolve_schema(schema, raw_prop).into_owned();
        let req = ov.required.contains(key.as_str());
        let req_label = if req { "required" } else { "optional" };
        let detail = format_property_detail(schema, key.as_str(), &prop_schema, 0);
        let desc = match &prop_schema {
            Schema::Object(o) => schema_object_metadata_description(o),
            _ => None,
        };
        let tail = match desc {
            Some(d) => format!(": {d}"),
            None => String::new(),
        };
        lines.push(format!(
            "- {key} ({detail}, {req_label}){tail}",
            key = key.as_str(),
            detail = detail,
            req_label = req_label,
            tail = tail
        ));
    }
    format!(
        "Tool \"{tool_name}\" rejected your arguments.\n\nError: {err_line}\n\nExpected arguments:\n{}\n\nRetry with corrected tool_calls.",
        lines.join("\n")
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

    use crate::tools::vault::write::VaultWriteArgs;
    use schemars::JsonSchema;
    use serde::Deserialize;

    #[test]
    fn natural_language_schema_simple_tool() {
        let schema = schemars::schema_for!(VaultWriteArgs);
        let out = natural_language_schema_description(
            "vault:write",
            &schema,
            "missing required field \"mode\"",
        );
        assert!(out.contains("vault:write"));
        assert!(out.contains("relative_path"));
        assert!(out.contains("content"));
        assert!(out.contains("mode"));
        assert!(out.contains("Expected arguments:"));
        assert!(out.contains("Retry with corrected tool_calls."));
    }

    #[derive(Debug, Deserialize, JsonSchema)]
    #[allow(dead_code)]
    struct OptArgs {
        required: String,
        #[serde(default)]
        maybe: Option<String>,
    }

    #[test]
    fn natural_language_schema_optional_fields() {
        let schema = schemars::schema_for!(OptArgs);
        let out = natural_language_schema_description("t:opt", &schema, "err");
        assert!(
            out.lines()
                .any(|line| line.contains("maybe") && line.contains("optional")),
            "{out}"
        );
        assert!(
            out.lines()
                .any(|line| line.starts_with("- required") && line.contains("required")),
            "{out}"
        );
    }

    #[test]
    fn natural_language_schema_enum_field() {
        let schema = schemars::schema_for!(VaultWriteArgs);
        let out = natural_language_schema_description("vault:write", &schema, "x");
        assert!(
            out.contains("overwrite") && out.contains("append"),
            "{out}"
        );
    }

    #[derive(Debug, Deserialize, JsonSchema)]
    struct EmptyArgs {}

    #[test]
    fn natural_language_schema_empty_args() {
        let schema = schemars::schema_for!(EmptyArgs);
        let out = natural_language_schema_description("t:noop", &schema, "bad");
        assert!(out.contains("No arguments required."));
    }
}
