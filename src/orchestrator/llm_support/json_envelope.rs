//! Split a raw LLM reply into the leading JSON object and any trailing text.

use crate::orchestrator::state::LlmResponse;
use schemars::schema::{
    InstanceType, RootSchema, Schema, SchemaObject, SingleOrVec,
};
use std::borrow::Cow;
use std::collections::HashSet;

/// First line of the JSON-parse recovery hint; also used to detect this path when shortening UI copy.
pub const FCP_JSON_REPAIR_MARKER: &str = "[FCP JSON REPAIR]";

/// One-line [`SessionEvent::SystemError`] when the model-facing message includes [`FCP_JSON_REPAIR_MARKER`].
pub const JSON_REPAIR_UI_SUMMARY: &str =
    crate::orchestrator::context::resolved_tool_recovery::JSON_REPAIR_TELEMETRY;

/// Human- and model-oriented hints appended after serde’s error when [`LlmResponse`] parsing fails.
/// Serde’s `expected ',' or '}'` near a `]` is often misread as a comma problem; this steers toward
/// the real issue (extra/missing `}` around `tool_calls` entries).
const LLM_JSON_PARSE_RECOVERY_HINT_BODY: &str = r##"The reply you just generated for this turn was not valid protocol JSON (single object: thought, status, message_to_user, tool_calls).

The serde error often means unbalanced { } braces — not “add a comma”. When `tool_calls` has exactly ONE item, a common mistake is: after the `}` that closes `args`, you must emit one more `}` to close the tool object before `]` ends the array.

Invalid: "tool_calls":[ {"name":"t","args":{...} ]
Valid:   "tool_calls":[ {"name":"t","args":{...} } ]

If the error says expected ',' or '}' at a line that shows `]`, you probably need that extra `}`.

Reply with one JSON object only (thought, status, message_to_user, tool_calls). No prose or markdown outside it."##;

/// When the model never entered JSON (prose, thinking-only, etc.): serde often reports `expected value at line 1 column 1`.
const LLM_JSON_PARSE_RECOVERY_HINT_NON_JSON_PREFIX: &str = r##"Your last reply was not FCP protocol JSON: it did not begin with a `{` object after optional `<think>` blocks.

Emit exactly one JSON object. The first non-space character of your reply must be `{`. Put all assistant prose in the string fields (`thought`, `message_to_user`), not before the object. No markdown code fences, no preamble, no trailing commentary outside the object."##;

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

/// `true` when the visible payload does not start a JSON object (after stripping Qwen-style thinking wrappers).
#[must_use]
pub fn raw_appears_to_start_without_json_object(raw: &str) -> bool {
    let t = strip_leading_redacted_thinking_block(raw).trim_start();
    !t.starts_with('{')
}

/// Byte length of a JSON string token starting at `s` (opening `"` included), or `None` if unterminated.
fn json_string_token_len(s: &str) -> Option<usize> {
    if !s.starts_with('"') {
        return None;
    }
    let bytes = s.as_bytes();
    let mut i = 1usize;
    let mut escape = false;
    while i < bytes.len() {
        let b = bytes[i];
        if escape {
            escape = false;
            i += 1;
            continue;
        }
        if b == b'\\' {
            escape = true;
            i += 1;
            continue;
        }
        if b == b'"' {
            return Some(i + 1);
        }
        i += 1;
    }
    None
}

/// Best-effort extraction of a JSON string field value after `"key":` in malformed output.
pub fn extract_json_string_field_best_effort(raw: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let idx = raw.find(&needle)?;
    let mut after = raw[idx + needle.len()..].trim_start();
    after = after.strip_prefix(':')?.trim_start();
    let len = json_string_token_len(after)?;
    serde_json::from_str(&after[..len]).ok()
}

/// Best-effort `message_to_user` when serde cannot parse the full protocol object.
pub fn extract_message_to_user_best_effort(raw: &str) -> Option<String> {
    extract_json_string_field_best_effort(raw, "message_to_user")
}

/// After at least one successful tool this turn, accept salvage when Idle was intended.
#[must_use]
pub fn raw_looks_like_idle_reply_intent(raw: &str) -> bool {
    let collapsed: String = raw
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let lower = collapsed.to_ascii_lowercase();
    lower.contains("\"status\":\"idle\"")
        || lower.contains("\"status\":\"reflect\"")
        || lower.contains("\"message_to_user\":\"")
}

/// Best-effort `message_to_user` salvage after tools ran (LlamaCpp trailing-brace faults).
#[must_use]
pub fn try_salvage_idle_message_after_tools(raw: &str, tool_rounds: u8) -> Option<String> {
    if tool_rounds == 0 {
        return None;
    }
    if !raw_looks_like_idle_reply_intent(raw) {
        return None;
    }
    extract_message_to_user_best_effort(raw).filter(|m| !m.trim().is_empty())
}

/// Best-effort extraction of `tool_calls[].name` values from malformed model output.
///
/// Used on Recover passes so the orchestrator can inject full JSON schemas for the tool(s) the
/// model was attempting — instead of slim phrase-map mode with a fresh semantic-router subset.
pub fn extract_tool_call_names_best_effort(raw: &str) -> Vec<String> {
    let mut names = Vec::<String>::new();
    let mut rest = raw;
    while let Some(i) = rest.find("\"name\"") {
        let mut after = rest[i + 6..].trim_start();
        if let Some(stripped) = after.strip_prefix(':') {
            after = stripped.trim_start();
        }
        let Some(inner) = after.strip_prefix('"') else {
            rest = &rest[i + 6..];
            continue;
        };
        let Some(end) = inner.find('"') else {
            rest = &rest[i + 6..];
            continue;
        };
        let name = &inner[..end];
        if name.contains(':') && !name.contains(char::is_whitespace) {
            if !names.iter().any(|n| n == name) {
                names.push(name.to_string());
            }
        }
        rest = &rest[i + 6..];
    }
    names
}

fn is_tool_name_token(name: &str) -> bool {
    if name.contains(char::is_whitespace) {
        return false;
    }
    let Some((ns, tool)) = name.split_once(':') else {
        return false;
    };
    !ns.is_empty()
        && !tool.is_empty()
        && ns
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
        && tool
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
}

fn push_unique_tool_name(names: &mut Vec<String>, name: &str) {
    if is_tool_name_token(name) && !names.iter().any(|n| n == name) {
        names.push(name.to_string());
    }
}

/// Scan `text` for bare `namespace:tool` tokens (lowercase ASCII).
fn scan_tool_name_tokens_in_text(text: &str, names: &mut Vec<String>) {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    while i < len {
        if !bytes[i].is_ascii_lowercase() {
            i += 1;
            continue;
        }
        let start = i;
        while i < len && (bytes[i].is_ascii_lowercase() || bytes[i].is_ascii_digit() || bytes[i] == b'_')
        {
            i += 1;
        }
        if i >= len || bytes[i] != b':' {
            continue;
        }
        i += 1;
        if i >= len || !bytes[i].is_ascii_lowercase() {
            continue;
        }
        while i < len && (bytes[i].is_ascii_lowercase() || bytes[i].is_ascii_digit() || bytes[i] == b'_')
        {
            i += 1;
        }
        push_unique_tool_name(names, &text[start..i]);
    }
}

fn extract_backtick_tool_names(text: &str, names: &mut Vec<String>) {
    let mut rest = text;
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else {
            break;
        };
        push_unique_tool_name(names, after[..end].trim());
        rest = &after[end + 1..];
    }
}

/// Best-effort extraction of tool names from model prose (not JSON `"name"` fields).
pub fn extract_tool_names_from_prose(raw: &str) -> Vec<String> {
    let mut names = Vec::<String>::new();
    if let Some(idx) = raw.find("Invoked tools:") {
        let after = &raw[idx + "Invoked tools:".len()..];
        let line = after.lines().next().unwrap_or(after);
        for part in line.split(',') {
            push_unique_tool_name(&mut names, part.trim());
        }
    }
    extract_backtick_tool_names(raw, &mut names);
    scan_tool_name_tokens_in_text(raw, &mut names);
    names
}

fn capped_router_hints(router_hints: &[String], cap: usize) -> Vec<String> {
    if router_hints.is_empty() {
        return Vec::new();
    }
    if cap == 0 {
        router_hints.to_vec()
    } else {
        router_hints.iter().take(cap).cloned().collect()
    }
}

/// Choose tool(s) for a Recover pass when protocol JSON parse fails or tools need full schemas.
///
/// Prefers the current turn's semantic router matches over stale tool-success lines in the chat stack.
pub fn select_recovery_targeted_tools(
    raw_llm_output: Option<&str>,
    user_message: &str,
    step_failed_tools: &[String],
    router_hints: &[String],
    chat_stack: &[crate::engine::Message],
    allowed: &HashSet<String>,
    router_cap: usize,
) -> Vec<String> {
    let raw = raw_llm_output.unwrap_or("");
    let non_json = raw_appears_to_start_without_json_object(raw);

    let mut candidates = if non_json {
        Vec::new()
    } else {
        extract_tool_call_names_best_effort(raw)
    };
    if candidates.is_empty() {
        candidates = extract_tool_names_from_prose(raw);
    }
    if candidates.is_empty() {
        candidates = infer_tools_from_user_message(user_message);
    }
    if candidates.is_empty() {
        candidates.extend(step_failed_tools.iter().cloned());
    }
    if candidates.is_empty() {
        candidates.extend(capped_router_hints(router_hints, router_cap));
    }
    if candidates.is_empty() {
        if let Some(name) = last_tool_name_from_chat_stack(chat_stack) {
            candidates.push(name);
        }
    }

    if step_failed_tools.is_empty() && !candidates.is_empty() && !router_hints.is_empty() {
        let capped = capped_router_hints(router_hints, router_cap);
        if let Some(top) = capped.first() {
            let any_in_router = candidates.iter().any(|c| capped.contains(c));
            if !any_in_router {
                candidates.insert(0, top.clone());
            }
        }
    }

    candidates.retain(|n| allowed.contains(n));
    candidates.sort();
    candidates.dedup();
    candidates
}

/// Infer tool names from the latest user turn (Recover fallback when JSON parse fails).
pub fn infer_tools_from_user_message(user: &str) -> Vec<String> {
    let lower = user.to_lowercase();
    let mut out = Vec::<String>::new();
    scan_tool_name_tokens_in_text(user, &mut out);
    let wants_find = lower.contains("web:find")
        || lower.contains("find on")
        || lower.contains("search within")
        || lower.contains("search the fetched")
        || lower.contains("search for")
        || lower.contains("match_count")
        || lower.contains("best_match_url")
        || lower.contains("bullet points from find");
    let wants_fetch = lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("web:fetch")
        || lower.contains("fetch http")
        || lower.contains("fetch https")
        || lower.contains("fetch www")
        || lower.contains("fetch that")
        || lower.contains("fetch the")
        || lower.contains("receipt_summary");
    let wants_news =
        lower.contains("news:today") || lower.contains("headline_count") || lower.contains("deep_fetch");
    if wants_find {
        out.push("web:find".into());
    }
    if wants_fetch {
        out.push("web:fetch".into());
    }
    if wants_news {
        out.push("news:today".into());
    }
    if (lower.contains("search") && lower.contains("vault")) || lower.contains("vault:search") {
        push_unique_tool_name(&mut out, "vault:search");
    }
    if lower.contains("list")
        && (lower.contains("folder")
            || lower.contains("vault")
            || lower.contains("director")
            || lower.contains("files"))
    {
        push_unique_tool_name(&mut out, "vault:list");
    }
    if (lower.contains("identity") || lower.contains("invariants"))
        && (lower.contains("read")
            || lower.contains("who are you")
            || lower.contains(".md")
            || lower.contains("vault"))
    {
        push_unique_tool_name(&mut out, "vault:read");
    }
    if lower.contains("health check")
        || lower.contains("system health")
        || lower.contains("system:health")
        || (lower.contains("health") && lower.contains("check"))
    {
        push_unique_tool_name(&mut out, "system:health");
    }
    if lower.contains("what time")
        || lower.contains("current time")
        || lower.contains("time is it")
        || lower.contains("clock:now")
    {
        push_unique_tool_name(&mut out, "clock:now");
    }
    if lower.contains("timer") || lower.contains("countdown") || lower.contains("clock:timer") {
        push_unique_tool_name(&mut out, "clock:timer");
    }
    if crate::orchestrator::llm_support::post_tool_guidance::user_wants_media_catalog(user) {
        push_unique_tool_name(&mut out, "media:catalog");
        push_unique_tool_name(&mut out, "vision:see");
    }
    if lower.contains("show me the image")
        || lower.contains("display the photo")
        || lower.contains("let me see it")
        || lower.contains("pull up that picture")
        || lower.contains("vision:display")
    {
        push_unique_tool_name(&mut out, "vision:display");
    }
    out
}

/// Last successful tool from the chat stack (system tool-success lines).
pub fn last_tool_name_from_chat_stack(stack: &[crate::engine::Message]) -> Option<String> {
    use crate::orchestrator::context::try_parse_tool_success_line;
    for msg in stack.iter().rev() {
        if msg.role == "system"
            && let Some(ts) = try_parse_tool_success_line(&msg.content)
            && ts.tool_name.contains(':')
        {
            return Some(ts.tool_name.to_string());
        }
    }
    None
}

/// Parse the leading JSON object as [`LlmResponse`] after tool-call normalization (same contract as the orchestrator directive path).
pub fn parse_llm_response_protocol(raw: &str) -> Result<LlmResponse, serde_json::Error> {
    let json_str = split_leading_json_object(raw).0;
    let mut parsed: LlmResponse = serde_json::from_str(json_str)?;
    parsed.normalize_tool_calls();
    Ok(parsed)
}

/// Full recovery payload for [`crate::orchestrator::state::LoopDirective::RecoverFromFuckup`].
pub fn llm_json_parse_recovery_message(err: &serde_json::Error, raw: &str) -> String {
    let hint_body = if raw_appears_to_start_without_json_object(raw) {
        LLM_JSON_PARSE_RECOVERY_HINT_NON_JSON_PREFIX
    } else {
        LLM_JSON_PARSE_RECOVERY_HINT_BODY
    };
    format!(
        "{err}\n\n{}\n{}",
        FCP_JSON_REPAIR_MARKER, hint_body
    )
}

/// Same as [`llm_json_parse_recovery_message`] plus a capped single-line excerpt of the raw model output (for the recovery LLM pass only).
pub fn llm_json_parse_recovery_message_with_excerpt(err: &serde_json::Error, raw: &str) -> String {
    let preview =
        capped_single_line_protocol_preview(raw, LLM_JSON_PARSE_RECOVERY_PREVIEW_MAX_CHARS);
    let hint_body = if raw_appears_to_start_without_json_object(raw) {
        LLM_JSON_PARSE_RECOVERY_HINT_NON_JSON_PREFIX
    } else {
        LLM_JSON_PARSE_RECOVERY_HINT_BODY
    };
    format!(
        "{err}\n\n{}\n{}\n\n[FCP: protocol_preview]\n{}",
        FCP_JSON_REPAIR_MARKER, hint_body, preview
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

/// Qwen / llama-server may emit a leading `<think>…</think>` block (sometimes
/// empty) before the FCP protocol JSON object. Strip that prefix so [`parse_llm_response_protocol`]
/// can see the JSON.
#[must_use]
pub fn strip_leading_redacted_thinking_block(raw: &str) -> &str {
    let trimmed = raw.trim_start();
    let Some(pos) = trimmed.find("</think>") else {
        return raw;
    };
    let after = trimmed[pos + "</think>".len()..].trim_start();
    if after.is_empty() {
        raw
    } else {
        after
    }
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
    fn strip_redacted_thinking_then_parse() {
        let raw = "<think>\n\n</think>\n{\"thought\":\"t\",\"status\":\"Idle\",\"message_to_user\":\"hi\",\"tool_calls\":[]}";
        let stripped = strip_leading_redacted_thinking_block(raw);
        assert!(stripped.starts_with('{'));
        let parsed = parse_llm_response_protocol(stripped).expect("parse");
        assert_eq!(parsed.thought, "t");
    }

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
    fn salvage_idle_message_when_tool_calls_truncated() {
        let raw = r#"{"thought":"t","status":"Idle","message_to_user":"Answer here.","tool_calls":[{"name":"web:find","args":{"query":"x"}"#;
        assert!(parse_llm_response_protocol(raw).is_err());
        assert!(raw_looks_like_idle_reply_intent(raw));
        assert_eq!(
            extract_message_to_user_best_effort(raw).as_deref(),
            Some("Answer here.")
        );
        let msg = try_salvage_idle_message_after_tools(raw, 2).expect("salvage");
        assert_eq!(msg, "Answer here.");
    }

    #[test]
    fn salvage_skipped_when_no_tools_ran() {
        let raw = r#"{"thought":"t","status":"Idle","message_to_user":"hi","tool_calls":[]}"#;
        assert!(try_salvage_idle_message_after_tools(raw, 0).is_none());
    }

    #[test]
    fn extract_tool_call_names_from_broken_tool_calls_array() {
        let raw = r#"{"thought":"t","status":"Task","tool_calls":[{"name":"web:fetch","args":{"url":"https://x"}]"#;
        let names = extract_tool_call_names_best_effort(raw);
        assert_eq!(names, vec!["web:fetch".to_string()]);
    }

    #[test]
    fn extract_tool_call_names_dedupes_and_skips_non_tools() {
        let raw = r#"{"name":"web:search","tool_calls":[{"name":"web:search","args":{}},{"name":"web:search","args":{}}]}"#;
        let names = extract_tool_call_names_best_effort(raw);
        assert_eq!(names, vec!["web:search".to_string()]);
    }

    #[test]
    fn extract_tool_names_from_prose_invoked_tools_line() {
        let raw = "Invoked tools: vault:search";
        let names = extract_tool_names_from_prose(raw);
        assert_eq!(names, vec!["vault:search".to_string()]);
    }

    #[test]
    fn extract_tool_names_from_prose_backtick_form() {
        let raw = "Please proceed with `vault:search` for these terms.";
        let names = extract_tool_names_from_prose(raw);
        assert_eq!(names, vec!["vault:search".to_string()]);
    }

    #[test]
    fn infer_tools_from_user_message_vault_search() {
        let names = infer_tools_from_user_message("Search the vault for mentions of synthesis");
        assert!(names.contains(&"vault:search".to_string()));
    }

    #[test]
    fn infer_tools_from_user_message_remember_image() {
        let names = infer_tools_from_user_message("remember this");
        assert!(names.contains(&"vision:see".to_string()));
        assert!(names.contains(&"media:catalog".to_string()));
    }

    #[test]
    fn select_recovery_router_beats_last_tool_on_chat_stack() {
        use crate::engine::Message;
        let allowed: HashSet<String> = [
            "clock:now",
            "vault:search",
            "vault:list",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let stack = vec![Message {
            role: "system".to_string(),
            content: "Tool 'clock:now' succeeded: SUCCESS: 16:00".to_string(),
        }];
        let router = vec![
            "vault:search".to_string(),
            "vault:list".to_string(),
        ];
        let user = "Search the vault for Talos and synthesis mentions.";
        let raw = "I am ready to search but have not executed yet.";
        let selected = select_recovery_targeted_tools(
            Some(raw),
            user,
            &[],
            &router,
            &stack,
            &allowed,
            0,
        );
        assert_eq!(selected, vec!["vault:search".to_string()]);
    }

    #[test]
    fn select_recovery_talos_turn8_regression() {
        use crate::engine::Message;
        let allowed: HashSet<String> = [
            "clock:now",
            "vault:search",
            "vault:list",
            "vault:read",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let stack = vec![Message {
            role: "system".to_string(),
            content: "Tool 'clock:now' succeeded: SUCCESS: current time".to_string(),
        }];
        let router = vec![
            "vault:search".to_string(),
            "vault:taglist".to_string(),
            "vault:list".to_string(),
        ];
        let user = "Search the vault for any notes mentioning synthesis or Talos.";
        let raw = "I am ready to search the vault for mentions of \"synthesis\" or \"Talos\", but I have not executed the search yet. Please confirm if you would like me to proceed with `vault:search` for these terms.";
        let selected = select_recovery_targeted_tools(
            Some(raw),
            user,
            &[],
            &router,
            &stack,
            &allowed,
            0,
        );
        assert_eq!(selected, vec!["vault:search".to_string()]);
    }

    #[test]
    fn select_recovery_failed_tool_precedence_over_last_tool() {
        use crate::engine::Message;
        let allowed: HashSet<String> = ["clock:now", "vault:read"]
            .into_iter()
            .map(String::from)
            .collect();
        let stack = vec![Message {
            role: "system".to_string(),
            content: "Tool 'clock:now' succeeded: SUCCESS".to_string(),
        }];
        let selected = select_recovery_targeted_tools(
            None,
            "Read my identity file",
            &["vault:read".to_string()],
            &["wiki:summary".to_string()],
            &stack,
            &allowed,
            0,
        );
        assert_eq!(selected, vec!["vault:read".to_string()]);
    }

    #[test]
    fn select_recovery_split_brain_prepends_router_top() {
        let allowed: HashSet<String> = ["clock:now", "vault:search"]
            .into_iter()
            .map(String::from)
            .collect();
        let raw = r#"{"thought":"search vault","status":"Task","tool_calls":[{"name":"clock:now","args":{}}]"#;
        let router = vec!["vault:search".to_string()];
        let selected = select_recovery_targeted_tools(
            Some(raw),
            "Search the vault for Talos",
            &[],
            &router,
            &[],
            &allowed,
            0,
        );
        assert_eq!(selected, vec!["clock:now".to_string(), "vault:search".to_string()]);
    }

    #[test]
    fn json_parse_recovery_message_includes_brace_and_tool_calls_hints() {
        let raw = r#"{"thought":"t","status":"Idle","message_to_user":"","tool_calls":[{"name":"t","args":{}"#;
        let err = parse_llm_response_protocol(raw).expect_err("invalid json");
        assert!(!raw_appears_to_start_without_json_object(raw));
        let msg = llm_json_parse_recovery_message(&err, raw);
        assert!(msg.contains("tool_calls"));
        assert!(msg.contains(FCP_JSON_REPAIR_MARKER));
        assert!(msg.contains("one more"));
    }

    #[test]
    fn json_parse_recovery_non_json_prefix_uses_leading_brace_hint() {
        let raw = "You've sharpened the blade.";
        let err = parse_llm_response_protocol(raw).expect_err("expected parse failure");
        assert!(raw_appears_to_start_without_json_object(raw));
        let msg = llm_json_parse_recovery_message_with_excerpt(&err, raw);
        assert!(msg.contains("first non-space"));
        assert!(!msg.contains("one more"));
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
