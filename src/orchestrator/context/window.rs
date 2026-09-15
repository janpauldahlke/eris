//! Sliding-window condensation: fold older `chat_stack` tail into a rolling JSON summary
//! while retaining recent messages under a token budget.

use crate::engine::Message;
use crate::executive::error::{FcpError, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

/// Assistant hop that requested native OpenAI `tool_calls` (OpenRouter round-trip).
pub fn is_native_tool_call_assistant(m: &Message) -> bool {
    m.role == "assistant" && !m.tool_calls.is_empty()
}

/// Native `role:tool` result frame.
pub fn is_native_tool_result(m: &Message) -> bool {
    m.role == "tool"
}

/// Row that belongs to a native tool round (assistant `tool_calls` hop or `role:tool` result).
pub fn is_native_tool_round_row(m: &Message) -> bool {
    is_native_tool_call_assistant(m) || is_native_tool_result(m)
}

fn native_round_end(messages: &[Message], start: usize) -> usize {
    let Some(first) = messages.get(start) else {
        return start;
    };
    let mut end = start;
    if is_native_tool_call_assistant(first) || is_native_tool_result(first) {
        while end + 1 < messages.len() && is_native_tool_result(&messages[end + 1]) {
            end = end.saturating_add(1);
        }
    }
    end
}

/// Inclusive `[start, end]` span of the native round containing `idx`, if any.
fn native_round_span(messages: &[Message], idx: usize) -> Option<(usize, usize)> {
    let m = messages.get(idx)?;
    if is_native_tool_call_assistant(m) {
        return Some((idx, native_round_end(messages, idx)));
    }
    if is_native_tool_result(m) {
        let mut start = idx;
        while start > 0 && is_native_tool_result(&messages[start - 1]) {
            start = start.saturating_sub(1);
        }
        if start > 0 && is_native_tool_call_assistant(&messages[start - 1]) {
            start = start.saturating_sub(1);
        }
        return Some((start, native_round_end(messages, start)));
    }
    None
}

/// How many leading tail rows to drop together so a native round stays paired.
fn native_round_drop_count(tail: &[Message]) -> usize {
    let Some(first) = tail.first() else {
        return 0;
    };
    if is_native_tool_call_assistant(first) || is_native_tool_result(first) {
        return native_round_end(tail, 0).saturating_add(1);
    }
    1
}

/// Move `split_at` so an assistant-with-`tool_calls` and its following `role:tool` frames
/// stay on the same side of the fold/keep boundary (OpenAI 400 if kept is unpaired).
fn snap_split_at_native_rounds(messages: &[Message], split_at: usize) -> usize {
    if split_at == 0 || split_at >= messages.len() {
        return split_at;
    }
    let mut span = None;
    for idx in [split_at.saturating_sub(1), split_at] {
        if let Some((start, end)) = native_round_span(messages, idx)
            && start < split_at
            && split_at <= end
        {
            span = Some((start, end));
            break;
        }
    }
    let Some((start, end)) = span else {
        return split_at;
    };
    if start == 0 {
        if end.saturating_add(1) >= messages.len() {
            return 0;
        }
        return end.saturating_add(1).min(messages.len());
    }
    start
}

fn split_at_native_snapped(tail: &[Message], split_at: usize) -> (Vec<Message>, Vec<Message>) {
    let split_at = snap_split_at_native_rounds(tail, split_at);
    (tail[..split_at].to_vec(), tail[split_at..].to_vec())
}

/// Stable identifier for the rolling summary (stack message content is JSON; not stored in ephemeral).
pub const ROLLING_SUMMARY_TITLE: &str = "fcp:rolling_context_summary";

pub const ROLLING_SUMMARY_KIND: &str = "rolling_summary_v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RollingSummaryV1 {
    pub kind: String,
    pub summary: String,
    #[serde(default)]
    pub key_facts: Vec<String>,
    #[serde(default)]
    pub open_threads: Vec<String>,
    pub last_updated: String,
}

impl RollingSummaryV1 {
    pub fn new(summary: String) -> Self {
        Self {
            kind: ROLLING_SUMMARY_KIND.to_string(),
            summary,
            key_facts: Vec::new(),
            open_threads: Vec::new(),
            last_updated: Utc::now().to_rfc3339(),
        }
    }
}

/// Cheap token proxy (no tokenizer in-tree).
///
/// Native assistant hops often have empty `content` and fat `tool_calls`; `role:tool`
/// frames carry `tool_call_id`. Both must count or proactive retain/trim under-counts.
pub fn estimate_message_tokens(m: &Message) -> usize {
    let mut n = m.content.chars().count();
    if let Some(id) = m.tool_call_id.as_deref() {
        n = n.saturating_add(id.chars().count());
    }
    if !m.tool_calls.is_empty() {
        match serde_json::to_string(&m.tool_calls) {
            Ok(serialized) => n = n.saturating_add(serialized.chars().count()),
            Err(_) => {
                for c in &m.tool_calls {
                    n = n.saturating_add(c.name.chars().count());
                    n = n.saturating_add(c.arguments.chars().count());
                    if let Some(id) = c.id.as_deref() {
                        n = n.saturating_add(id.chars().count());
                    }
                }
            }
        }
    }
    (n / 4).saturating_add(1).saturating_add(4)
}

pub fn estimate_stack_tokens(messages: &[Message]) -> usize {
    messages.iter().map(estimate_message_tokens).sum()
}

pub fn is_jit_system_message(m: &Message) -> bool {
    m.role == "system" && m.content.starts_with("[JIT TOOL GUIDANCE]")
}

fn json_slice(s: &str) -> &str {
    if let (Some(start), Some(end)) = (s.find('{'), s.rfind('}')) {
        if start <= end { &s[start..=end] } else { s }
    } else {
        s
    }
}

pub fn is_rolling_summary_message(m: &Message) -> bool {
    if m.role != "system" {
        return false;
    }
    let slice = json_slice(m.content.trim());
    serde_json::from_str::<RollingSummaryV1>(slice)
        .map(|r| r.kind == ROLLING_SUMMARY_KIND)
        .unwrap_or(false)
}

#[derive(Debug, Clone)]
pub struct StackHead {
    pub main_system: Message,
    pub jit: Option<Message>,
    pub rolling: Option<Message>,
}

/// Splits the stack into the fixed head (main system, optional JIT, optional rolling summary)
/// and the conversational/tool tail.
pub fn split_stack_head(stack: &[Message]) -> Result<StackHead> {
    let Some(main_system) = stack.first().cloned() else {
        return Err(FcpError::EngineFault(
            "context window: empty chat stack".to_string(),
        ));
    };
    if main_system.role != "system" {
        tracing::warn!("context window: first message is not system role");
    }
    let mut i = 1usize;
    let jit = if i < stack.len() && is_jit_system_message(&stack[i]) {
        let m = stack[i].clone();
        i = i.saturating_add(1);
        Some(m)
    } else {
        None
    };
    let rolling = if i < stack.len() && is_rolling_summary_message(&stack[i]) {
        Some(stack[i].clone())
    } else {
        None
    };
    Ok(StackHead {
        main_system,
        jit,
        rolling,
    })
}

pub fn tail_after_head(stack: &[Message], head: &StackHead) -> Vec<Message> {
    let mut n = 1usize;
    if head.jit.is_some() {
        n = n.saturating_add(1);
    }
    if head.rolling.is_some() {
        n = n.saturating_add(1);
    }
    stack.iter().skip(n).cloned().collect()
}

/// Max estimated tokens to keep verbatim in the tail (recent window).
pub fn retain_budget_tokens(num_ctx: usize, retain_ratio: f32) -> usize {
    let n = num_ctx.max(1);
    let r = retain_ratio.clamp(0.05_f32, 0.95_f32);
    ((n as f32) * r).floor() as usize
}

/// Split `tail` into (older messages to fold, recent messages to keep).
///
/// When the tail contains any `user` message, the **last** `user` message and every message after
/// it are always kept (the active human request plus the model’s reply/tool work for that turn).
/// Older prefix bytes are budgeted separately so a long assistant trace cannot evict the latest
/// user line from the verbatim tail.
pub fn split_tail_fold_and_keep(tail: &[Message], budget: usize) -> (Vec<Message>, Vec<Message>) {
    if tail.is_empty() {
        return (Vec::new(), Vec::new());
    }
    match tail.iter().rposition(|m| m.role == "user") {
        Some(last_user_idx) => {
            split_tail_fold_and_keep_with_last_user_anchor(tail, last_user_idx, budget)
        }
        None => split_tail_fold_and_keep_no_user_anchor(tail, budget),
    }
}

fn split_tail_fold_and_keep_with_last_user_anchor(
    tail: &[Message],
    last_user_idx: usize,
    budget: usize,
) -> (Vec<Message>, Vec<Message>) {
    let suffix = &tail[last_user_idx..];
    let suffix_tokens = estimate_stack_tokens(suffix);
    let prefix = &tail[..last_user_idx];
    let prefix_budget = budget.saturating_sub(suffix_tokens);

    let mut kept_from_prefix_rev: Vec<Message> = Vec::new();
    let mut used = 0usize;
    for m in prefix.iter().rev() {
        let t = estimate_message_tokens(m);
        if used.saturating_add(t) > prefix_budget && !kept_from_prefix_rev.is_empty() {
            break;
        }
        used = used.saturating_add(t);
        kept_from_prefix_rev.push(m.clone());
    }
    kept_from_prefix_rev.reverse();
    let kept_prefix_len = kept_from_prefix_rev.len();
    let split_at = prefix.len().saturating_sub(kept_prefix_len);
    split_at_native_snapped(tail, split_at)
}

fn split_tail_fold_and_keep_no_user_anchor(
    tail: &[Message],
    budget: usize,
) -> (Vec<Message>, Vec<Message>) {
    let mut kept: Vec<Message> = Vec::new();
    let mut used = 0usize;
    for m in tail.iter().rev() {
        let t = estimate_message_tokens(m);
        if used.saturating_add(t) > budget && !kept.is_empty() {
            break;
        }
        used = used.saturating_add(t);
        kept.push(m.clone());
    }
    kept.reverse();
    let split_at = tail.len().saturating_sub(kept.len());

    if split_at == 0 && tail.len() >= 2 {
        let n_fold = (tail.len().saturating_sub(1))
            .min(tail.len().div_ceil(3))
            .max(1);
        return split_at_native_snapped(tail, n_fold);
    }

    split_at_native_snapped(tail, split_at)
}

/// Plan for one condensation pass: one LLM call folds `messages_to_fold` into new rolling JSON.
#[derive(Debug, Clone)]
pub struct CondensationPlan {
    pub main_system: Message,
    pub jit: Option<Message>,
    /// Prior rolling JSON string (from stack or ephemeral), for the summarizer.
    pub previous_rolling_json: Option<String>,
    pub messages_to_fold: Vec<Message>,
    pub kept_tail: Vec<Message>,
}

/// Build a condensation plan, or `None` if there is nothing worth folding (no LLM call).
pub fn plan_sliding_condensation(
    stack: &[Message],
    num_ctx: usize,
    retain_ratio: f32,
) -> Result<Option<CondensationPlan>> {
    let head = split_stack_head(stack)?;
    let tail = tail_after_head(stack, &head);
    if tail.is_empty() {
        return Ok(None);
    }

    let previous_rolling_json = head
        .rolling
        .as_ref()
        .map(|m| m.content.clone())
        .filter(|s| !s.trim().is_empty());

    let budget = retain_budget_tokens(num_ctx, retain_ratio).max(32);
    let (messages_to_fold, kept_tail) = split_tail_fold_and_keep(&tail, budget);

    if messages_to_fold.is_empty() {
        return Ok(None);
    }

    Ok(Some(CondensationPlan {
        main_system: head.main_system,
        jit: head.jit,
        previous_rolling_json,
        messages_to_fold,
        kept_tail,
    }))
}

/// Drop oldest tail messages (after the fixed head) until the estimated stack is at most `ceiling`,
/// without removing the latest `user` message or anything after it.
pub fn trim_chat_stack_to_est_token_ceiling(
    stack: &mut Vec<Message>,
    ceiling: usize,
) -> Result<usize> {
    let mut dropped = 0usize;
    if ceiling == 0 {
        return Ok(0);
    }
    while estimate_stack_tokens(stack) > ceiling {
        let head = split_stack_head(stack)?;
        let n_head = 1 + usize::from(head.jit.is_some()) + usize::from(head.rolling.is_some());
        if stack.len() <= n_head {
            break;
        }
        let last_user_rel = stack[n_head..].iter().rposition(|m| m.role == "user");
        let drop_count = native_round_drop_count(&stack[n_head..]).max(1);
        let last_drop_idx = n_head.saturating_add(drop_count).saturating_sub(1);
        let removed = match last_user_rel {
            Some(rel) => {
                let abs = n_head + rel;
                if abs > last_drop_idx {
                    for _ in 0..drop_count {
                        stack.remove(n_head);
                    }
                    drop_count
                } else {
                    0
                }
            }
            None => {
                if stack.len() > last_drop_idx.saturating_add(1) {
                    for _ in 0..drop_count {
                        stack.remove(n_head);
                    }
                    drop_count
                } else {
                    0
                }
            }
        };
        if removed == 0 {
            break;
        }
        dropped = dropped.saturating_add(removed);
    }
    Ok(dropped)
}

pub fn condensation_system_instruction() -> String {
    format!(
        "You fold older conversation into ONE compact rolling summary.\n\
         Output a single JSON object only. No markdown fences. No extra text.\n\
         Required shape:\n\
         {{\n\
           \"kind\": \"{kind}\",\n\
           \"summary\": \"concise narrative of what happened in the folded messages\",\n\
           \"key_facts\": [\"short bullet facts\"],\n\
           \"open_threads\": [\"unresolved items\"],\n\
           \"last_updated\": \"RFC3339 timestamp\"\n\
         }}\n\
         Merge prior rolling summary (if provided) with the new messages; do not drop critical constraints.\n\
         If any folded `user` lines exist, copy the **latest human request / goal** into `open_threads` or `key_facts` \
         in clear, quotable form so the assistant still knows what the user is trying to accomplish after compaction.",
        kind = ROLLING_SUMMARY_KIND
    )
}

/// Build a small stack for the summarization-only LLM call (not the full agent stack).
pub fn build_summarization_stack(
    instruction: String,
    previous_rolling_json: Option<&str>,
    messages_to_fold: &[Message],
) -> Vec<Message> {
    let mut out = vec![Message::system(instruction)];
    if let Some(prev) = previous_rolling_json.filter(|s| !s.trim().is_empty()) {
        out.push(Message::system(format!(
            "[PRIOR_ROLLING_SUMMARY_JSON]\n{prev}\n[/PRIOR_ROLLING_SUMMARY_JSON]"
        )));
    }
    for m in messages_to_fold {
        out.push(m.clone());
    }
    out
}

/// llama-server + Qwen3 chat template can raise `No user query found in messages` when the
/// wire `messages` array contains no `user` role, or when the last message is not `user`
/// (condensation stacks are often `system…` + folded `assistant` rows only). Append a
/// single internal user line so the template always has an explicit query to answer.
pub fn ensure_condensation_user_query_tail(stack: &mut Vec<Message>) {
    let last_is_user = stack.last().is_some_and(|m| m.role == "user");
    if last_is_user {
        return;
    }
    stack.push(Message::user("[FCP internal — condensation] Reply with exactly one JSON object as specified in the system instructions (rolling_summary_v1). No markdown fences, no prose before or after the object."));
}

pub fn normalize_rolling_summary_response(raw: &str) -> Result<String> {
    let slice = json_slice(raw.trim());
    let mut v: RollingSummaryV1 = match serde_json::from_str(slice) {
        Ok(v) => v,
        Err(_) => RollingSummaryV1::new(slice.to_string()),
    };
    if v.kind != ROLLING_SUMMARY_KIND {
        v.kind = ROLLING_SUMMARY_KIND.to_string();
    }
    if v.last_updated.trim().is_empty() {
        v.last_updated = Utc::now().to_rfc3339();
    }
    serde_json::to_string(&v).map_err(FcpError::from)
}

pub fn rolling_summary_system_message(json: &str) -> Message {
    Message::system(json.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_head_orders_jit_then_rolling() {
        let stack = vec![
            Message::system("main".to_string()),
            Message::system("[JIT TOOL GUIDANCE]\nx\n[/JIT TOOL GUIDANCE]".to_string()),
            Message::system(r#"{"kind":"rolling_summary_v1","summary":"s","key_facts":[],"open_threads":[],"last_updated":"2026-01-01T00:00:00+00:00"}"#.to_string()),
            Message::user("hi".to_string()),
        ];
        let head = split_stack_head(&stack).expect("split");
        assert!(head.jit.is_some());
        assert!(head.rolling.is_some());
        let tail = tail_after_head(&stack, &head);
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].content, "hi");
    }

    #[test]
    fn retain_keeps_suffix_under_budget() {
        let tail: Vec<Message> = (0u8..6)
            .map(|i| {
                Message::new(
                    if i % 2 == 0 {
                        crate::engine::Role::User
                    } else {
                        crate::engine::Role::Assistant
                    },
                    "word ".repeat(20),
                )
            })
            .collect();
        let budget = 50usize;
        let (old, kept) = split_tail_fold_and_keep(&tail, budget);
        assert!(!old.is_empty());
        assert!(!kept.is_empty());
        assert_eq!(old.len() + kept.len(), tail.len());
    }

    #[test]
    fn retain_budget_respects_ratio() {
        assert_eq!(retain_budget_tokens(1000, 0.55), 550);
        assert_eq!(retain_budget_tokens(1000, 0.2), 200);
    }

    #[test]
    fn hard_trim_drops_oldest_tail_under_ceiling() {
        let main = Message::system("main");
        let u1 = Message::user("x".repeat(400));
        let a1 = Message::assistant("y".repeat(400));
        let u2 = Message::user("current");
        let mut stack = vec![main, u1, a1, u2];
        let ceiling = 80usize;
        let dropped = trim_chat_stack_to_est_token_ceiling(&mut stack, ceiling).expect("trim");
        assert!(dropped > 0);
        assert!(estimate_stack_tokens(&stack) <= ceiling);
        assert!(
            stack.iter().any(|m| m.content == "current"),
            "latest user line preserved"
        );
    }

    #[test]
    fn split_keeps_last_user_turn_even_with_heavy_assistant_suffix() {
        let heavy = "w".repeat(500);
        let tail = vec![
            Message::user("stale ask"),
            Message::assistant(heavy.clone()),
            Message::assistant(heavy.clone()),
            Message::user("CURRENT_USER_GOAL"),
            Message::assistant(heavy.clone()),
            Message::assistant("tiny"),
        ];
        let budget = 120usize;
        let (old, kept) = split_tail_fold_and_keep(&tail, budget);
        let joined: String = kept.iter().map(|m| m.content.as_str()).collect();
        assert!(
            joined.contains("CURRENT_USER_GOAL"),
            "expected latest user line in kept tail; old={old:?} kept={kept:?}"
        );
        assert!(
            !old.iter().any(|m| m.content == "CURRENT_USER_GOAL"),
            "latest user line must not be folded; old={old:?}"
        );
    }

    #[test]
    fn condensation_user_tail_appended_when_last_not_user() {
        use super::ensure_condensation_user_query_tail;
        let mut stack = vec![Message::system("instr"), Message::assistant("{}")];
        ensure_condensation_user_query_tail(&mut stack);
        assert_eq!(stack.len(), 3);
        assert_eq!(stack.last().map(|m| m.role.as_str()), Some("user"));
    }

    #[test]
    fn condensation_user_tail_skipped_when_already_user() {
        use super::ensure_condensation_user_query_tail;
        let mut stack = vec![Message::system("instr"), Message::user("hi")];
        ensure_condensation_user_query_tail(&mut stack);
        assert_eq!(stack.len(), 2);
    }

    fn native_round(query: &str) -> Vec<Message> {
        vec![
            Message::assistant_with_tool_calls(
                "",
                vec![crate::engine::EngineToolCall {
                    id: Some("call_1".into()),
                    name: "memory:query".into(),
                    arguments: format!(r#"{{"query":"{query}"}}"#),
                }],
            ),
            Message::tool("Tool 'memory:query' succeeded: hit", "call_1"),
        ]
    }

    fn assert_native_rounds_paired(side: &[Message]) {
        let mut i = 0;
        while i < side.len() {
            if is_native_tool_call_assistant(&side[i]) {
                let end = {
                    let mut e = i;
                    while e + 1 < side.len() && is_native_tool_result(&side[e + 1]) {
                        e += 1;
                    }
                    e
                };
                assert!(
                    end > i,
                    "assistant tool_calls must keep following role:tool on the same side"
                );
                i = end + 1;
                continue;
            }
            assert!(
                !is_native_tool_result(&side[i]),
                "orphan role:tool on a fold/keep side"
            );
            assert!(!is_native_tool_round_row(&side[i]) || is_native_tool_call_assistant(&side[i]));
            i += 1;
        }
    }

    #[test]
    fn estimate_counts_native_tool_calls_not_just_content() {
        let empty = Message::assistant("");
        let native = Message::assistant_with_tool_calls(
            "",
            vec![crate::engine::EngineToolCall {
                id: Some("call_1".into()),
                name: "memory:query".into(),
                arguments: r#"{"query":"abcdefghijklmnop"}"#.into(),
            }],
        );
        assert!(
            estimate_message_tokens(&native) > estimate_message_tokens(&empty),
            "native tool_calls must add to the token proxy"
        );
        let tool = Message::tool("ok", "call_1_long_id");
        let tool_short = Message::tool("ok", "x");
        assert!(estimate_message_tokens(&tool) > estimate_message_tokens(&tool_short));
    }

    #[test]
    fn fold_does_not_split_native_assistant_from_role_tool() {
        let mut tail = vec![Message::user("stale")];
        tail.extend(native_round("old"));
        tail.push(Message::user("CURRENT"));
        tail.extend(native_round("new"));
        let budget = 40usize;
        let (old, kept) = split_tail_fold_and_keep(&tail, budget);
        assert_eq!(old.len() + kept.len(), tail.len());
        assert_native_rounds_paired(&old);
        assert_native_rounds_paired(&kept);
        assert!(kept.iter().any(|m| m.content == "CURRENT"));
    }

    #[test]
    fn trim_drops_native_round_as_one_unit() {
        let main = Message::system("main");
        let fat_args = "q".repeat(800);
        let assistant = Message::assistant_with_tool_calls(
            "",
            vec![crate::engine::EngineToolCall {
                id: Some("call_1".into()),
                name: "memory:query".into(),
                arguments: format!(r#"{{"query":"{fat_args}"}}"#),
            }],
        );
        let tool = Message::tool(format!("Tool 'memory:query' succeeded: {}", "z".repeat(800)), "call_1");
        let u2 = Message::user("current");
        let mut stack = vec![main, assistant, tool, u2];
        let before = stack.len();
        let dropped = trim_chat_stack_to_est_token_ceiling(&mut stack, 80).expect("trim");
        assert!(dropped >= 2, "must drop assistant+tool together, dropped={dropped}");
        assert!(stack.iter().any(|m| m.content == "current"));
        assert!(
            !stack.iter().any(is_native_tool_result)
                || stack.iter().any(is_native_tool_call_assistant),
            "must not leave orphan role:tool after trim; stack={stack:?}"
        );
        assert!(stack.len() < before);
    }
}
