//! LLM-only view of [`crate::engine::Message`] history: the stored [`crate::orchestrator::core::Orchestrator::chat_stack`]
//! remains canonical; this module produces a transformed copy for [`crate::engine::LlmEngine::generate`].

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::engine::Message;
use crate::tools::ToolContextViewHint;

/// Policy knobs for [`build_llm_view`], wired from [`crate::config::AppConfig`] at startup.
#[derive(Debug, Clone)]
pub struct ContextViewSettings {
    pub enabled: bool,
    pub default_snippet_chars: usize,
    pub assistant_compact: bool,
    pub hints: Arc<HashMap<String, ToolContextViewHint>>,
}

impl Default for ContextViewSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            default_snippet_chars: 400,
            assistant_compact: true,
            hints: Arc::new(HashMap::new()),
        }
    }
}

fn extract_json_slice(response_json: &str) -> &str {
    if let (Some(start), Some(end)) = (response_json.find('{'), response_json.rfind('}')) {
        if start <= end {
            &response_json[start..=end]
        } else {
            response_json
        }
    } else {
        response_json
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

fn try_parse_tool_success_line(content: &str) -> Option<(&str, &str)> {
    const PREFIX: &str = "Tool '";
    let rest = content.strip_prefix(PREFIX)?;
    let (name, body) = rest.split_once("' succeeded: ")?;
    Some((name, body))
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
    let slice = extract_json_slice(content);
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
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn approx_stack_chars(messages: &[Message]) -> usize {
    messages.iter().map(|m| m.content.chars().count()).sum()
}

/// Build a copy of `messages` for the LLM. When disabled, returns a shallow clone of the slice.
pub fn build_llm_view(messages: &[Message], settings: &ContextViewSettings) -> Vec<Message> {
    if !settings.enabled {
        return messages.to_vec();
    }

    let hints = settings.hints.as_ref();
    let before = approx_stack_chars(messages);
    let mut rewritten = 0usize;
    let mut out: Vec<Message> = Vec::with_capacity(messages.len());

    for m in messages {
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
            && let Some((tool_name, body)) = try_parse_tool_success_line(&m.content)
        {
            let hint = hints
                .get(tool_name)
                .copied()
                .unwrap_or(ToolContextViewHint::Default);
            let new_content =
                rewrite_tool_line(&m.content, tool_name, body, hint, settings.default_snippet_chars);
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
            messages_total = messages.len(),
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

    fn hint_map(names: &[(&str, ToolContextViewHint)]) -> Arc<HashMap<String, ToolContextViewHint>> {
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
            hints: Arc::new(HashMap::new()),
        };
        let v = build_llm_view(&m, &settings);
        assert_eq!(v[0].role, "assistant");
        assert!(v[0].content.contains("Hello"));
        assert!(v[0].content.contains("Invoked tools: a:b"));
        assert!(!v[0].content.contains("thought"));
    }

    #[test]
    fn assistant_parse_failure_keeps_original() {
        let m = vec![Message {
            role: "assistant".to_string(),
            content: "not json at all".to_string(),
        }];
        let settings = ContextViewSettings {
            enabled: true,
            default_snippet_chars: 400,
            assistant_compact: true,
            hints: Arc::new(HashMap::new()),
        };
        let v = build_llm_view(&m, &settings);
        assert_eq!(v[0].content, "not json at all");
    }
}
