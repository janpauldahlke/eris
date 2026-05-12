//! LLM-only view of [`crate::engine::Message`] history: the stored [`crate::orchestrator::core::Orchestrator::chat_stack`]
//! remains canonical; this module produces a transformed copy for [`crate::engine::LlmEngine::generate`].

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::engine::Message;
use crate::orchestrator::llm_support::json_envelope::{
    parse_llm_response_protocol, split_leading_json_object,
};
use crate::tools::ToolContextViewHint;

use super::resolved_tool_recovery::apply_omit_resolved_tool_recovery;
use super::stack_lines::try_parse_tool_success_line;

/// Start delimiter for the JSON tool-definition array inside the assembled system prompt ([`crate::orchestrator::context::ContextAssembler::build_tool_prompt`]).
pub const FCP_TOOL_DEFS_BEGIN: &str = "<<<FCP_TOOL_DEFS_JSON>>>";
/// End delimiter for that JSON block (paired with [`FCP_TOOL_DEFS_BEGIN`]).
pub const FCP_TOOL_DEFS_END: &str = "<<<END_FCP_TOOL_DEFS_JSON>>>";

const FCP_TOOL_DEFS_DISCLOSURE: &str = "[FCP] Tool parameter JSON schemas are omitted below to save context; full schemas are enforced server-side. Use exact tool names and valid args; invalid calls are rejected.";

const LOG_TOOL_NAMES_MAX_CHARS: usize = 2000;

fn cap_log_string(s: &str, max_chars: usize) -> String {
    let n = s.chars().count();
    if n <= max_chars {
        return s.to_string();
    }
    let take = max_chars.saturating_sub(20);
    s.chars().take(take).collect::<String>() + "… [truncated]"
}

/// Metadata for [`fcp.context_view`] when tool definitions are slimmed for the LLM view.
#[derive(Debug, Clone)]
pub struct SlimToolDefsMeta {
    pub tools_offered_count: usize,
    pub tools_offered_names_for_log: String,
}

/// Strip `function.parameters` from each OpenAI-style tool entry and prepend disclosure + name list.
/// Used only in the LLM-facing view when [`ContextViewSettings::full_tool_schemas_in_llm_view`] is false.
pub fn slim_tool_definitions_inner(inner_json: &str) -> Result<(String, SlimToolDefsMeta), String> {
    let v: Value = serde_json::from_str(inner_json.trim())
        .map_err(|e| format!("tool defs JSON parse: {e}"))?;
    let arr = v
        .as_array()
        .ok_or_else(|| "tool defs: expected JSON array".to_string())?;
    let mut names: Vec<String> = Vec::new();
    let mut slim: Vec<Value> = Vec::with_capacity(arr.len());
    for item in arr {
        let mut item = item.clone();
        if let Some(n) = item
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(|n| n.as_str())
        {
            names.push(n.to_string());
        }
        if let Some(func) = item.get_mut("function").and_then(|f| f.as_object_mut()) {
            let _ = func.remove("parameters");
        }
        slim.push(item);
    }
    let pretty =
        serde_json::to_string_pretty(&slim).map_err(|e| format!("tool defs serialize: {e}"))?;
    let count = names.len();
    let names_joined = names.join(", ");
    let names_for_log = cap_log_string(&names_joined, LOG_TOOL_NAMES_MAX_CHARS);
    let mut out = String::with_capacity(
        FCP_TOOL_DEFS_DISCLOSURE.len() + 64 + pretty.len() + names_joined.len(),
    );
    out.push_str(FCP_TOOL_DEFS_DISCLOSURE);
    out.push_str("\n\n");
    out.push_str(&format!("Tools in view ({count}): {names_joined}\n\n"));
    out.push_str(&pretty);
    Ok((
        out,
        SlimToolDefsMeta {
            tools_offered_count: count,
            tools_offered_names_for_log: names_for_log,
        },
    ))
}

fn try_slim_tool_definitions_in_system_content(
    content: &str,
) -> Option<(String, SlimToolDefsMeta)> {
    if !content.contains(FCP_TOOL_DEFS_BEGIN) || !content.contains(FCP_TOOL_DEFS_END) {
        return None;
    }
    let start = content.find(FCP_TOOL_DEFS_BEGIN)?;
    let after_begin = start + FCP_TOOL_DEFS_BEGIN.len();
    let tail = content.get(after_begin..)?;
    let end_rel = tail.find(FCP_TOOL_DEFS_END)?;
    let inner_end = after_begin + end_rel;
    let inner = content.get(after_begin..inner_end)?;
    let inner_trim = inner.trim();
    match slim_tool_definitions_inner(inner_trim) {
        Ok((slim_inner, meta)) => {
            let mut new_content = String::with_capacity(content.len());
            new_content.push_str(content.get(..start)?);
            new_content.push_str(FCP_TOOL_DEFS_BEGIN);
            new_content.push('\n');
            new_content.push_str(&slim_inner);
            new_content.push('\n');
            new_content.push_str(FCP_TOOL_DEFS_END);
            let after_end = inner_end + FCP_TOOL_DEFS_END.len();
            new_content.push_str(content.get(after_end..).unwrap_or(""));
            Some((new_content, meta))
        }
        Err(e) => {
            tracing::warn!(
                target: "fcp.context_view",
                error = %e,
                "slim tool definitions failed; keeping full block in LLM view"
            );
            None
        }
    }
}

/// Policy knobs for [`build_llm_view`], wired from [`crate::config::AppConfig`] at startup.
#[derive(Debug, Clone)]
pub struct ContextViewSettings {
    pub enabled: bool,
    pub default_snippet_chars: usize,
    pub assistant_compact: bool,
    /// When false and [`Self::enabled`] is true, strip `parameters` from tool defs in the LLM view only.
    pub full_tool_schemas_in_llm_view: bool,
    /// When true and [`Self::enabled`] is true, collapse resolved tool-recovery spans before successful tool batches in the LLM view only.
    pub omit_resolved_tool_recovery: bool,
    /// When true and [`Self::enabled`] is true, replace assistant rows that are not valid protocol JSON with a short placeholder (canonical stack unchanged).
    pub assistant_non_json_placeholder: bool,
    pub hints: Arc<HashMap<String, ToolContextViewHint>>,
}

impl Default for ContextViewSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            default_snippet_chars: 400,
            assistant_compact: true,
            full_tool_schemas_in_llm_view: false,
            omit_resolved_tool_recovery: true,
            assistant_non_json_placeholder: true,
            hints: Arc::new(HashMap::new()),
        }
    }
}

fn trim_snippet(input: &str, max_len: usize) -> String {
    if input.len() <= max_len {
        return input.to_string();
    }
    let mut limit = max_len;
    while limit > 0 && !input.is_char_boundary(limit) {
        limit -= 1;
    }
    let mut out = input[..limit].to_string();
    out.push_str("… [truncated]");
    out
}

fn rewrite_tool_line(
    full_line: &str,
    tool_name: &str,
    body: &str,
    hint: ToolContextViewHint,
    default_snippet_chars: usize,
) -> String {
    match hint {
        ToolContextViewHint::Full => full_line.to_string(),
        ToolContextViewHint::MarkerOnly => format!("[tool] {tool_name} ok"),
        ToolContextViewHint::Default => {
            let t = trim_snippet(body, default_snippet_chars);
            format!("[tool] {tool_name} ok\n{t}")
        }
        ToolContextViewHint::Snippet { max_chars } => {
            let t = trim_snippet(body, max_chars);
            format!("[tool] {tool_name} ok\n{t}")
        }
    }
}

fn compact_assistant_json(content: &str) -> Option<String> {
    let slice = split_leading_json_object(content).0;
    let v: Value = serde_json::from_str(slice).ok()?;
    let msg = v
        .get("message_to_user")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .unwrap_or("");
    let names: Vec<String> = v
        .get("tool_calls")
        .and_then(|tc| tc.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();

    let mut out = String::new();
    if !msg.is_empty() {
        out.push_str(msg);
    }
    if !names.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("Invoked tools: ");
        out.push_str(&names.join(", "));
    }
    if out.is_empty() { None } else { Some(out) }
}

fn approx_stack_chars(messages: &[Message]) -> usize {
    messages.iter().map(|m| m.content.chars().count()).sum()
}

/// Build a copy of `messages` for the LLM. When disabled, returns a shallow clone of the slice.
pub fn build_llm_view(messages: &[Message], settings: &ContextViewSettings) -> Vec<Message> {
    if !settings.enabled {
        return messages.to_vec();
    }

    let folded: Option<Vec<Message>> = if settings.omit_resolved_tool_recovery {
        Some(apply_omit_resolved_tool_recovery(messages))
    } else {
        None
    };
    let source: &[Message] = match &folded {
        Some(v) => v.as_slice(),
        None => messages,
    };

    let hints = settings.hints.as_ref();
    let before = approx_stack_chars(source);
    let mut rewritten = 0usize;
    let mut out: Vec<Message> = Vec::with_capacity(source.len());

    for m in source {
        if m.role == "assistant"
            && settings.assistant_non_json_placeholder
            && parse_llm_response_protocol(&m.content).is_err()
        {
            let n = m.content.chars().count();
            rewritten += 1;
            out.push(Message {
                role: m.role.clone(),
                content: format!("[FCP: non-protocol assistant output omitted; {n} chars]"),
            });
            continue;
        }

        if m.role == "assistant"
            && settings.assistant_compact
            && let Some(compact) = compact_assistant_json(&m.content)
        {
            if compact != m.content {
                rewritten += 1;
            }
            out.push(Message {
                role: m.role.clone(),
                content: compact,
            });
            continue;
        }

        if m.role == "system"
            && !settings.full_tool_schemas_in_llm_view
            && let Some((new_content, meta)) =
                try_slim_tool_definitions_in_system_content(&m.content)
        {
            let before_len = m.content.len();
            let after_len = new_content.len();
            if new_content != m.content {
                rewritten += 1;
            }
            tracing::info!(
                target: "fcp.context_view",
                tool_defs_view_mode = "slim",
                tools_offered_count = meta.tools_offered_count,
                tools_offered_names = %meta.tools_offered_names_for_log,
                tool_defs_chars_before = before_len,
                tool_defs_chars_after = after_len,
                tool_defs_chars_saved = before_len.saturating_sub(after_len),
                "tool definitions slimmed for LLM view"
            );
            out.push(Message {
                role: m.role.clone(),
                content: new_content,
            });
            continue;
        }

        if m.role == "system"
            && let Some(ts) = try_parse_tool_success_line(&m.content)
        {
            let tool_name = ts.tool_name;
            let body = ts.body;
            let hint = hints
                .get(tool_name)
                .copied()
                .unwrap_or(ToolContextViewHint::Default);
            let new_content = rewrite_tool_line(
                &m.content,
                tool_name,
                body,
                hint,
                settings.default_snippet_chars,
            );
            if new_content != m.content {
                rewritten += 1;
            }
            out.push(Message {
                role: m.role.clone(),
                content: new_content,
            });
            continue;
        }

        out.push(m.clone());
    }

    let after = approx_stack_chars(&out);
    if before != after || rewritten > 0 {
        tracing::info!(
            target: "fcp.context_view",
            messages_total = source.len(),
            messages_rewritten = rewritten,
            approx_chars_before = before,
            approx_chars_after = after,
            "context view applied for LLM.generate"
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::context::resolved_tool_recovery::OMIT_RESOLVED_TOOL_RECOVERY_PLACEHOLDER;
    use crate::orchestrator::context::stack_lines::format_tool_success_line;

    fn hint_map(
        names: &[(&str, ToolContextViewHint)],
    ) -> Arc<HashMap<String, ToolContextViewHint>> {
        Arc::new(
            names
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect::<HashMap<_, _>>(),
        )
    }

    #[test]
    fn disabled_passes_through() {
        let m = vec![Message {
            role: "system".to_string(),
            content: "Tool 'x:y' succeeded: hello".to_string(),
        }];
        let settings = ContextViewSettings::default();
        let v = build_llm_view(&m, &settings);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].content, m[0].content);
    }

    #[test]
    fn tool_line_snippet_default() {
        let body = "a".repeat(500);
        let m = vec![Message {
            role: "system".to_string(),
            content: format!("Tool 't:1' succeeded: {body}"),
        }];
        let settings = ContextViewSettings {
            enabled: true,
            default_snippet_chars: 100,
            assistant_compact: false,
            full_tool_schemas_in_llm_view: false,
            omit_resolved_tool_recovery: false,
            assistant_non_json_placeholder: false,
            hints: hint_map(&[("t:1", ToolContextViewHint::Default)]),
        };
        let v = build_llm_view(&m, &settings);
        assert!(v[0].content.contains("[tool] t:1 ok"));
        assert!(v[0].content.contains("… [truncated]"));
        assert!(!v[0].content.contains(&body));
    }

    #[test]
    fn tool_line_full_keeps_original() {
        let line = "Tool 't:2' succeeded: payload".to_string();
        let m = vec![Message {
            role: "system".to_string(),
            content: line.clone(),
        }];
        let settings = ContextViewSettings {
            enabled: true,
            default_snippet_chars: 10,
            assistant_compact: false,
            full_tool_schemas_in_llm_view: false,
            omit_resolved_tool_recovery: false,
            assistant_non_json_placeholder: false,
            hints: hint_map(&[("t:2", ToolContextViewHint::Full)]),
        };
        let v = build_llm_view(&m, &settings);
        assert_eq!(v[0].content, line);
    }

    #[test]
    fn tool_line_marker_only() {
        let m = vec![Message {
            role: "system".to_string(),
            content: "Tool 't:3' succeeded: huge".to_string(),
        }];
        let settings = ContextViewSettings {
            enabled: true,
            default_snippet_chars: 400,
            assistant_compact: false,
            full_tool_schemas_in_llm_view: false,
            omit_resolved_tool_recovery: false,
            assistant_non_json_placeholder: false,
            hints: hint_map(&[("t:3", ToolContextViewHint::MarkerOnly)]),
        };
        let v = build_llm_view(&m, &settings);
        assert_eq!(v[0].content, "[tool] t:3 ok");
    }

    #[test]
    fn assistant_compact_strips_json_noise() {
        let raw = r#"{"thought":"x","status":"Reflect","message_to_user":"Hello","tool_calls":[{"name":"a:b","args":{}}]}"#;
        let m = vec![Message {
            role: "assistant".to_string(),
            content: raw.to_string(),
        }];
        let settings = ContextViewSettings {
            enabled: true,
            default_snippet_chars: 400,
            assistant_compact: true,
            full_tool_schemas_in_llm_view: false,
            omit_resolved_tool_recovery: false,
            assistant_non_json_placeholder: false,
            hints: Arc::new(HashMap::new()),
        };
        let v = build_llm_view(&m, &settings);
        assert_eq!(v[0].role, "assistant");
        assert!(v[0].content.contains("Hello"));
        assert!(v[0].content.contains("Invoked tools: a:b"));
        assert!(!v[0].content.contains("thought"));
    }

    #[test]
    fn assistant_parse_failure_keeps_original_when_placeholder_disabled() {
        let m = vec![Message {
            role: "assistant".to_string(),
            content: "not json at all".to_string(),
        }];
        let settings = ContextViewSettings {
            enabled: true,
            default_snippet_chars: 400,
            assistant_compact: true,
            full_tool_schemas_in_llm_view: false,
            omit_resolved_tool_recovery: false,
            assistant_non_json_placeholder: false,
            hints: Arc::new(HashMap::new()),
        };
        let v = build_llm_view(&m, &settings);
        assert_eq!(v[0].content, "not json at all");
    }

    #[test]
    fn assistant_parse_failure_rewrites_with_placeholder_when_enabled() {
        let body = "not json at all";
        let m = vec![Message {
            role: "assistant".to_string(),
            content: body.to_string(),
        }];
        let settings = ContextViewSettings {
            enabled: true,
            default_snippet_chars: 400,
            assistant_compact: true,
            full_tool_schemas_in_llm_view: false,
            omit_resolved_tool_recovery: false,
            assistant_non_json_placeholder: true,
            hints: Arc::new(HashMap::new()),
        };
        let v = build_llm_view(&m, &settings);
        let n = body.chars().count();
        assert_eq!(
            v[0].content,
            format!("[FCP: non-protocol assistant output omitted; {n} chars]")
        );
    }

    #[test]
    fn slim_tool_definitions_inner_strips_parameters() {
        let inner = r#"[{"type":"function","function":{"name":"vault:read","description":"read","parameters":{"type":"object","properties":{"path":{"type":"string"}}}}}]"#;
        let (slim, meta) = slim_tool_definitions_inner(inner).expect("slim");
        assert!(!slim.contains("\"parameters\""));
        assert!(slim.contains("vault:read"));
        assert!(slim.contains("[FCP] Tool parameter JSON schemas are omitted"));
        assert_eq!(meta.tools_offered_count, 1);
        assert!(meta.tools_offered_names_for_log.contains("vault:read"));
    }

    #[test]
    fn build_llm_view_slims_delimited_tool_block() {
        let json = r#"[{"type":"function","function":{"name":"t:x","description":"hi","parameters":{"type":"object"}}}]"#;
        let full = format!(
            "prefix\n{begin}\n{json}\n{end}\nsuffix",
            begin = FCP_TOOL_DEFS_BEGIN,
            json = json,
            end = FCP_TOOL_DEFS_END,
        );
        let m = vec![Message {
            role: "system".to_string(),
            content: full,
        }];
        let settings = ContextViewSettings {
            enabled: true,
            default_snippet_chars: 400,
            assistant_compact: false,
            full_tool_schemas_in_llm_view: false,
            omit_resolved_tool_recovery: false,
            assistant_non_json_placeholder: false,
            hints: Arc::new(HashMap::new()),
        };
        let v = build_llm_view(&m, &settings);
        assert!(!v[0].content.contains("\"parameters\""));
        assert!(v[0].content.contains("t:x"));
        assert!(v[0].content.contains("Tools in view (1):"));
    }

    #[test]
    fn build_llm_view_respects_full_tool_schemas_flag() {
        let json = r#"[{"type":"function","function":{"name":"t:x","description":"hi","parameters":{"type":"object"}}}]"#;
        let full = format!(
            "p\n{begin}\n{json}\n{end}\n",
            begin = FCP_TOOL_DEFS_BEGIN,
            json = json,
            end = FCP_TOOL_DEFS_END,
        );
        let m = vec![Message {
            role: "system".to_string(),
            content: full,
        }];
        let settings = ContextViewSettings {
            enabled: true,
            default_snippet_chars: 400,
            assistant_compact: false,
            full_tool_schemas_in_llm_view: true,
            omit_resolved_tool_recovery: false,
            assistant_non_json_placeholder: false,
            hints: Arc::new(HashMap::new()),
        };
        let v = build_llm_view(&m, &settings);
        assert!(v[0].content.contains("\"parameters\""));
    }

    #[test]
    fn omit_resolved_tool_recovery_then_tool_snippet_still_applies() {
        let stack = vec![
            Message {
                role: "user".to_string(),
                content: "hi".to_string(),
            },
            Message {
                role: "assistant".to_string(),
                content: "bad".to_string(),
            },
            Message {
                role: "system".to_string(),
                content: "[SYSTEM] Invalid model output: x".to_string(),
            },
            Message {
                role: "assistant".to_string(),
                content: r#"{"tool_calls":[{"name":"t:1","args":{}}]}"#.to_string(),
            },
            Message {
                role: "system".to_string(),
                content: format_tool_success_line("t:1", &"z".repeat(300)),
            },
        ];
        let settings = ContextViewSettings {
            enabled: true,
            default_snippet_chars: 80,
            assistant_compact: false,
            full_tool_schemas_in_llm_view: false,
            omit_resolved_tool_recovery: true,
            assistant_non_json_placeholder: false,
            hints: hint_map(&[("t:1", ToolContextViewHint::Default)]),
        };
        let v = build_llm_view(&stack, &settings);
        assert_eq!(
            v.iter()
                .filter(|m| m.content == OMIT_RESOLVED_TOOL_RECOVERY_PLACEHOLDER)
                .count(),
            1
        );
        let tool_line = v.iter().find(|m| m.content.contains("[tool] t:1 ok"));
        assert!(tool_line.is_some());
        assert!(
            tool_line
                .expect("tool line")
                .content
                .contains("… [truncated]")
        );
    }
}
