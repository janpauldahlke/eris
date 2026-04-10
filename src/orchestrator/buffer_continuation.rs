//! Detect recent staged large content (`vault:read`, `web:fetch`) or `ephemeral:buffer_page` results
//! so routing and short-input guards keep tool mode and embed a **verbatim buffer handle + paging cursor**.

use crate::engine::Message;
use crate::orchestrator::context::{parse_system_line, ParsedSystemLine};
use serde_json::Value;

const SNIPPET_MAX_CHARS: usize = 2000;
const SCAN_MAX_MESSAGES: usize = 28;

/// `true` if the stack has material for buffer follow-up routing (staging receipt and/or a recent page read).
pub fn stack_has_buffer_routing_context(stack: &[Message]) -> bool {
    buffer_followup_routing_appendix(stack).is_some()
}

/// Back-compat name for call sites that mean “staged or paging in progress”.
#[inline]
pub fn stack_has_staged_buffer_receipt(stack: &[Message]) -> bool {
    stack_has_buffer_routing_context(stack)
}

/// Text appended to the semantic-router embedding: prefer a structured cursor from the latest
/// `ephemeral:buffer_page`, else a truncated vault / web staging snippet.
pub fn buffer_followup_routing_appendix(stack: &[Message]) -> Option<String> {
    if let Some(block) = format_buffer_page_session_block(stack) {
        return Some(block);
    }
    staged_vault_or_web_snippet(stack)
}

/// Back-compat: same as [`buffer_followup_routing_appendix`].
#[inline]
pub fn recent_staged_buffer_snippet(stack: &[Message]) -> Option<String> {
    buffer_followup_routing_appendix(stack)
}

/// Build `[FCP BUFFER SESSION]` from the most recent successful `ephemeral:buffer_page` JSON body.
fn format_buffer_page_session_block(stack: &[Message]) -> Option<String> {
    for m in stack.iter().rev().take(SCAN_MAX_MESSAGES) {
        if m.role != "system" {
            continue;
        }
        let ParsedSystemLine::ToolSuccess(ts) = parse_system_line(&m.content) else {
            continue;
        };
        if ts.tool_name != "ephemeral:buffer_page" {
            continue;
        }
        let body = ts.body.trim();
        if !body.starts_with('{') {
            continue;
        }
        let v: Value = serde_json::from_str(body).ok()?;
        let buffer_id = v.get("buffer_id").and_then(|x| x.as_str())?;
        let page = v.get("page").and_then(|x| x.as_u64())? as usize;
        let page_size = v
            .get("page_size")
            .and_then(|x| x.as_u64())
            .unwrap_or(1) as usize;
        let page_count = v.get("page_count").and_then(|x| x.as_u64())? as usize;
        let total_chunks = v
            .get("total_chunks")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let source = v
            .get("source")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let next_page = v
            .get("next_page")
            .and_then(|x| x.as_u64())
            .map(|n| n as usize)
            .or_else(|| {
                if page + 1 < page_count {
                    Some(page + 1)
                } else {
                    None
                }
            });
        let next_hint = next_page
            .map(|n| format!("{}", n))
            .unwrap_or_else(|| "(none — last page for this page_size)".to_string());
        return Some(format!(
            "[FCP BUFFER SESSION — copy `buffer_id` exactly (e.g. buf_1); use `next_page` for ephemeral:buffer_page]\n\
buffer_id: {buffer_id}\n\
source: {source}\n\
last_page: {page}\n\
page_size: {page_size}\n\
page_count: {page_count}\n\
total_chunks: {total_chunks}\n\
next_page: {next_hint}\n\
[/FCP BUFFER SESSION]"
        ));
    }
    None
}

fn staged_vault_or_web_snippet(stack: &[Message]) -> Option<String> {
    for m in stack.iter().rev().take(SCAN_MAX_MESSAGES) {
        if m.role != "system" {
            continue;
        }
        let ParsedSystemLine::ToolSuccess(ts) = parse_system_line(&m.content) else {
            continue;
        };
        if ts.tool_name == "vault:read" && ts.body.contains("Large vault file staged as ephemeral buffer") {
            return Some(truncate_chars(&ts.body, SNIPPET_MAX_CHARS));
        }
        if ts.tool_name == "web:fetch" {
            let body = ts.body.trim();
            if !body.starts_with('{') {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
                continue;
            };
            let chunk_count = v
                .get("chunk_count")
                .and_then(|c| c.as_u64())
                .unwrap_or(0);
            if v.get("artifact_id").and_then(|a| a.as_str()).is_some() && chunk_count > 1 {
                return Some(truncate_chars(body, SNIPPET_MAX_CHARS));
            }
        }
    }
    None
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::context::format_tool_success_line;

    #[test]
    fn finds_vault_staging_snippet() {
        let stack = vec![Message {
            role: "system".into(),
            content: format_tool_success_line(
                "vault:read",
                "[Large vault file staged as ephemeral buffer]\n\n{\"buffer_id\":\"abc\"}\n",
            ),
        }];
        assert!(stack_has_buffer_routing_context(&stack));
        let s = buffer_followup_routing_appendix(&stack).expect("snippet");
        assert!(s.contains("buffer_id"));
    }

    #[test]
    fn buffer_page_alone_provides_routing_context() {
        let body = r#"{"buffer_id":"buf_1","source":"x.md","page":1,"page_size":2,"page_count":3,"total_chunks":5,"next_page":2,"chunks":[]}"#;
        let stack = vec![Message {
            role: "system".into(),
            content: format_tool_success_line("ephemeral:buffer_page", body),
        }];
        assert!(stack_has_buffer_routing_context(&stack));
        let s = buffer_followup_routing_appendix(&stack).expect("appendix");
        assert!(s.contains("buf_1"));
        assert!(s.contains("last_page: 1"));
        assert!(s.contains("next_page: 2"));
    }

    #[test]
    fn finds_web_multi_chunk() {
        let stack = vec![Message {
            role: "system".into(),
            content: format_tool_success_line(
                "web:fetch",
                r#"{"artifact_id":"x","chunk_count":3,"preview_head":"p"}"#,
            ),
        }];
        assert!(stack_has_buffer_routing_context(&stack));
    }

    #[test]
    fn ignores_single_chunk_web() {
        let stack = vec![Message {
            role: "system".into(),
            content: format_tool_success_line(
                "web:fetch",
                r#"{"artifact_id":"x","chunk_count":1,"preview_head":"p"}"#,
            ),
        }];
        assert!(!stack_has_buffer_routing_context(&stack));
    }
}
