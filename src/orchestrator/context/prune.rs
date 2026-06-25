//! Targeted pruning of stale tool results on the canonical [`crate::engine::Message`] chat stack.
//!
//! Unlike [`super::view`] (which creates a transient LLM-only copy), functions here **mutate** the
//! stored stack so that memory is actually reclaimed and condensation pressure drops.
//!
//! Current policy: keep only the **most recent** success line for a given tool name; older
//! results are replaced with a compact marker that preserves breadcrumb continuity for the model.

use crate::engine::Message;
use super::stack_lines::try_parse_tool_success_line;

/// Replace all but the most recent `tool_name` success result with a compact marker.
///
/// Returns the number of messages that were pruned (0 when there is at most one match).
/// The marker preserves the tool name so the model can still see the read-trail.
pub fn prune_stale_tool_results(
    chat_stack: &mut Vec<Message>,
    tool_name: &str,
    keep_last_n: usize,
) -> usize {
    let match_indices: Vec<usize> = chat_stack
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m.role == "system"
                && try_parse_tool_success_line(&m.content)
                    .map(|ts| ts.tool_name == tool_name)
                    .unwrap_or(false)
        })
        .map(|(i, _)| i)
        .collect();

    if match_indices.len() <= keep_last_n {
        return 0;
    }

    let prune_count = match_indices.len() - keep_last_n;
    let to_prune = &match_indices[..prune_count];
    let mut pruned = 0usize;

    for &idx in to_prune {
        let original = &chat_stack[idx].content;
        let snippet = try_parse_tool_success_line(original)
            .map(|ts| ts.body)
            .unwrap_or("");
        let preview: String = snippet.chars().take(60).collect();
        let chars_freed = original.len();
        chat_stack[idx].content =
            format!("[{tool_name}: result pruned from context ({chars_freed} chars); began with: {preview}…]");
        pruned += 1;
    }

    if pruned > 0 {
        tracing::info!(
            event = "fcp.context.prune_stale_tool_results",
            tool = tool_name,
            pruned,
            kept = keep_last_n,
            "Pruned stale tool results from chat stack"
        );
    }

    pruned
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::context::format_tool_success_line;

    fn sys(content: &str) -> Message {
        Message {
            role: "system".to_string(),
            content: content.to_string(),
        }
    }

    fn assistant(content: &str) -> Message {
        Message {
            role: "assistant".to_string(),
            content: content.to_string(),
        }
    }

    #[test]
    fn prunes_oldest_keeps_latest() {
        let body_a = "Document: book (chunks 0-14 of 100)\n\nchunk content A";
        let body_b = "Document: book (chunks 15-29 of 100)\n\nchunk content B";
        let body_c = "Document: book (chunks 30-44 of 100)\n\nchunk content C";
        let mut stack = vec![
            sys("system prompt"),
            assistant("first response"),
            sys(&format_tool_success_line("doc:read", body_a)),
            sys("post-tool guidance"),
            assistant("second response"),
            sys(&format_tool_success_line("doc:read", body_b)),
            sys("post-tool guidance"),
            assistant("third response"),
            sys(&format_tool_success_line("doc:read", body_c)),
        ];

        let pruned = prune_stale_tool_results(&mut stack, "doc:read", 1);
        assert_eq!(pruned, 2);

        assert!(stack[2].content.contains("[doc:read: result pruned"));
        assert!(stack[5].content.contains("[doc:read: result pruned"));

        assert!(stack[8]
            .content
            .starts_with("Tool 'doc:read' succeeded:"));
    }

    #[test]
    fn no_prune_when_single_result() {
        let mut stack = vec![
            sys("system prompt"),
            sys(&format_tool_success_line("doc:read", "only one")),
        ];
        let pruned = prune_stale_tool_results(&mut stack, "doc:read", 1);
        assert_eq!(pruned, 0);
        assert!(stack[1].content.contains("only one"));
    }

    #[test]
    fn does_not_touch_other_tools() {
        let mut stack = vec![
            sys(&format_tool_success_line("vault:read", "vault content")),
            sys(&format_tool_success_line("doc:read", "doc A")),
            sys(&format_tool_success_line("doc:read", "doc B")),
        ];
        let pruned = prune_stale_tool_results(&mut stack, "doc:read", 1);
        assert_eq!(pruned, 1);
        assert!(
            stack[0].content.contains("vault content"),
            "vault:read must be untouched"
        );
    }

    #[test]
    fn keep_last_two() {
        let mut stack = vec![
            sys(&format_tool_success_line("doc:read", "A")),
            sys(&format_tool_success_line("doc:read", "B")),
            sys(&format_tool_success_line("doc:read", "C")),
        ];
        let pruned = prune_stale_tool_results(&mut stack, "doc:read", 2);
        assert_eq!(pruned, 1);
        assert!(stack[0].content.contains("[doc:read: result pruned"));
        assert!(stack[1].content.starts_with("Tool 'doc:read' succeeded:"));
        assert!(stack[2].content.starts_with("Tool 'doc:read' succeeded:"));
    }
}
