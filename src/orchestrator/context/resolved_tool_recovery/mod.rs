//! Collapse resolved tool-recovery spans in the LLM-facing view only ([`super::build_llm_view`]).

mod markers;

pub use markers::{
    DUPLICATE_TOOL_BATCH_PREFIX, JSON_REPAIR_TELEMETRY, PROTOCOL_FAULT_PREFIX,
    SYSTEM_RECOVERY_PREFIX, is_recovery_system_content,
};

use crate::engine::Message;

use super::stack_lines::try_parse_tool_success_line;
use super::window::is_jit_system_message;

/// Single-line system stub replacing omitted recovery transcript in the LLM view.
pub const OMIT_RESOLVED_TOOL_RECOVERY_PLACEHOLDER: &str =
    "[FCP] Resolved tool-recovery attempts omitted from LLM view; see telemetry.";

fn is_tool_success_message(m: &Message) -> bool {
    m.role == "system" && try_parse_tool_success_line(&m.content).is_some()
}

/// Indices strictly before `w` to remove when they form a recovery episode ending at winning assistant `w`.
fn indices_to_collapse_before_w(messages: &[Message], w: usize) -> Option<Vec<usize>> {
    if w == 0 {
        return None;
    }
    let mut out: Vec<usize> = Vec::new();
    let mut saw_recovery = false;
    let mut i = w.checked_sub(1)?;
    loop {
        let m = messages.get(i)?;
        if m.role == "user" {
            break;
        }
        if m.role == "assistant" {
            out.push(i);
            i = match i.checked_sub(1) {
                Some(v) => v,
                None => break,
            };
            continue;
        }
        if m.role == "system" {
            if is_jit_system_message(m) {
                i = match i.checked_sub(1) {
                    Some(v) => v,
                    None => break,
                };
                continue;
            }
            if markers::is_recovery_system_content(&m.content) {
                out.push(i);
                saw_recovery = true;
                i = match i.checked_sub(1) {
                    Some(v) => v,
                    None => break,
                };
                continue;
            }
            break;
        }
        break;
    }
    if !saw_recovery || out.is_empty() {
        return None;
    }
    out.sort_unstable();
    out.dedup();
    Some(out)
}

fn collect_tool_success_runs(messages: &[Message]) -> Vec<(usize, usize, usize)> {
    let mut runs: Vec<(usize, usize, usize)> = Vec::new();
    let mut idx = 0usize;
    while idx < messages.len() {
        if !is_tool_success_message(&messages[idx]) {
            idx += 1;
            continue;
        }
        let tool_start = idx;
        let mut j = idx;
        while j < messages.len() && is_tool_success_message(&messages[j]) {
            j += 1;
        }
        if let Some(w) = tool_start.checked_sub(1)
            && messages.get(w).is_some_and(|m| m.role == "assistant")
        {
            runs.push((w, tool_start, j));
        }
        idx = j;
    }
    runs
}

/// Returns a copy of `messages` with resolved recovery spans before successful tool batches replaced by one placeholder system line each.
pub fn apply_omit_resolved_tool_recovery(messages: &[Message]) -> Vec<Message> {
    let runs = collect_tool_success_runs(messages);
    if runs.is_empty() {
        return messages.to_vec();
    }
    let mut remove: Vec<usize> = Vec::new();
    for &(w, _, _) in runs.iter().rev() {
        if let Some(mut indices) = indices_to_collapse_before_w(messages, w) {
            remove.append(&mut indices);
        }
    }
    if remove.is_empty() {
        return messages.to_vec();
    }
    remove.sort_unstable();
    remove.dedup();
    let remove_set: std::collections::HashSet<usize> = remove.iter().copied().collect();
    let mut removed_chars = 0usize;
    for &i in &remove {
        if let Some(m) = messages.get(i) {
            removed_chars = removed_chars.saturating_add(m.content.len());
        }
    }
    tracing::info!(
        target: "fcp.context_view",
        removed_message_count = remove.len(),
        removed_approx_chars = removed_chars,
        "resolved tool-recovery span omitted from LLM view"
    );
    let mut out: Vec<Message> = Vec::with_capacity(messages.len());
    let mut i = 0usize;
    while i < messages.len() {
        if !remove_set.contains(&i) {
            out.push(messages[i].clone());
            i += 1;
            continue;
        }
        out.push(Message {
            role: "system".to_string(),
            content: OMIT_RESOLVED_TOOL_RECOVERY_PLACEHOLDER.to_string(),
        });
        while i < messages.len() && remove_set.contains(&i) {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::context::stack_lines::format_tool_success_line;

    fn sys(s: &str) -> Message {
        Message {
            role: "system".to_string(),
            content: s.to_string(),
        }
    }

    fn asst(s: &str) -> Message {
        Message {
            role: "assistant".to_string(),
            content: s.to_string(),
        }
    }

    fn user(s: &str) -> Message {
        Message {
            role: "user".to_string(),
            content: s.to_string(),
        }
    }

    #[test]
    fn noop_when_clean_assistant_then_tool_success() {
        let stack = vec![
            user("hi"),
            asst(r#"{"tool_calls":[{"name":"t:x","args":{}}]}"#),
            sys(&format_tool_success_line("t:x", "ok")),
        ];
        let out = apply_omit_resolved_tool_recovery(&stack);
        assert_eq!(out.len(), stack.len());
        assert!(
            !out.iter()
                .any(|m| m.content == OMIT_RESOLVED_TOOL_RECOVERY_PLACEHOLDER)
        );
    }

    #[test]
    fn collapses_recovery_before_winning_assistant() {
        let stack = vec![
            user("hi"),
            asst("bad"),
            sys("[SYSTEM] Invalid model output: x"),
            asst(r#"{"tool_calls":[{"name":"t:x","args":{}}]}"#),
            sys(&format_tool_success_line("t:x", "ok")),
        ];
        let out = apply_omit_resolved_tool_recovery(&stack);
        assert!(
            out.iter()
                .filter(|m| m.content == OMIT_RESOLVED_TOOL_RECOVERY_PLACEHOLDER)
                .count()
                == 1
        );
        assert!(!out.iter().any(|m| m.content == "bad"));
        assert!(
            out.iter()
                .any(|m| m.content.contains("tool_calls") && m.role == "assistant")
        );
        assert!(
            out.iter()
                .any(|m| m.content == format_tool_success_line("t:x", "ok"))
        );
    }

    #[test]
    fn retains_jit_before_winning_assistant() {
        let jit = "[JIT TOOL GUIDANCE]\nx\n[/JIT TOOL GUIDANCE]";
        let stack = vec![
            user("hi"),
            asst("bad"),
            sys("[SYSTEM] Recovery — schema"),
            sys(jit),
            asst(r#"{"tool_calls":[{"name":"t:x","args":{}}]}"#),
            sys(&format_tool_success_line("t:x", "ok")),
        ];
        let out = apply_omit_resolved_tool_recovery(&stack);
        assert!(out.iter().any(|m| m.content == jit));
        assert!(
            out.iter()
                .filter(|m| m.content == OMIT_RESOLVED_TOOL_RECOVERY_PLACEHOLDER)
                .count()
                == 1
        );
    }

    #[test]
    fn stops_at_user_boundary() {
        let stack = vec![
            user("first"),
            asst("old"),
            user("second"),
            asst("bad"),
            sys("[SYSTEM] Invalid model output: x"),
            asst(r#"{"tool_calls":[{"name":"t:x","args":{}}]}"#),
            sys(&format_tool_success_line("t:x", "ok")),
        ];
        let out = apply_omit_resolved_tool_recovery(&stack);
        assert!(out.iter().any(|m| m.content == "old"));
    }
}
