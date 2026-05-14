//! Sliding-window condensation: fold older `chat_stack` tail into a rolling JSON summary
//! while retaining recent messages under a token budget.

use crate::engine::Message;
use crate::executive::error::{FcpError, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

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
pub fn estimate_message_tokens(m: &Message) -> usize {
    let base = (m.content.chars().count() / 4).saturating_add(1);
    base.saturating_add(4)
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
    let old_part = prefix[..split_at].to_vec();
    let mut kept_tail = Vec::with_capacity(kept_from_prefix_rev.len() + suffix.len());
    kept_tail.extend(kept_from_prefix_rev);
    kept_tail.extend_from_slice(suffix);
    (old_part, kept_tail)
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
    let mut old_part = tail[..split_at].to_vec();
    let mut kept_tail = kept;

    if old_part.is_empty() && tail.len() >= 2 {
        let n_fold = (tail.len().saturating_sub(1))
            .min((tail.len() + 2) / 3)
            .max(1);
        old_part = tail[..n_fold].to_vec();
        kept_tail = tail[n_fold..].to_vec();
    }

    (old_part, kept_tail)
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
        let removed_one = match last_user_rel {
            Some(rel) => {
                let abs = n_head + rel;
                if abs > n_head {
                    stack.remove(n_head);
                    true
                } else {
                    false
                }
            }
            None => {
                if stack.len() > n_head + 1 {
                    stack.remove(n_head);
                    true
                } else {
                    false
                }
            }
        };
        if !removed_one {
            break;
        }
        dropped = dropped.saturating_add(1);
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
    let mut out = vec![Message {
        role: "system".to_string(),
        content: instruction,
    }];
    if let Some(prev) = previous_rolling_json.filter(|s| !s.trim().is_empty()) {
        out.push(Message {
            role: "system".to_string(),
            content: format!("[PRIOR_ROLLING_SUMMARY_JSON]\n{prev}\n[/PRIOR_ROLLING_SUMMARY_JSON]"),
        });
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
    let last_is_user = stack
        .last()
        .is_some_and(|m| m.role == "user");
    if last_is_user {
        return;
    }
    stack.push(Message {
        role: "user".into(),
        content: "[FCP internal — condensation] Reply with exactly one JSON object as specified in the system instructions (rolling_summary_v1). No markdown fences, no prose before or after the object.".into(),
    });
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
    Message {
        role: "system".to_string(),
        content: json.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_head_orders_jit_then_rolling() {
        let stack = vec![
            Message {
                role: "system".to_string(),
                content: "main".to_string(),
            },
            Message {
                role: "system".to_string(),
                content: "[JIT TOOL GUIDANCE]\nx\n[/JIT TOOL GUIDANCE]".to_string(),
            },
            Message {
                role: "system".to_string(),
                content: r#"{"kind":"rolling_summary_v1","summary":"s","key_facts":[],"open_threads":[],"last_updated":"2026-01-01T00:00:00+00:00"}"#.to_string(),
            },
            Message {
                role: "user".to_string(),
                content: "hi".to_string(),
            },
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
            .map(|i| Message {
                role: if i % 2 == 0 { "user" } else { "assistant" }.to_string(),
                content: "word ".repeat(20),
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
        let main = Message {
            role: "system".into(),
            content: "main".into(),
        };
        let u1 = Message {
            role: "user".into(),
            content: "x".repeat(400),
        };
        let a1 = Message {
            role: "assistant".into(),
            content: "y".repeat(400),
        };
        let u2 = Message {
            role: "user".into(),
            content: "current".into(),
        };
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
            Message {
                role: "user".into(),
                content: "stale ask".into(),
            },
            Message {
                role: "assistant".into(),
                content: heavy.clone(),
            },
            Message {
                role: "assistant".into(),
                content: heavy.clone(),
            },
            Message {
                role: "user".into(),
                content: "CURRENT_USER_GOAL".into(),
            },
            Message {
                role: "assistant".into(),
                content: heavy.clone(),
            },
            Message {
                role: "assistant".into(),
                content: "tiny".into(),
            },
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
        let mut stack = vec![
            Message {
                role: "system".into(),
                content: "instr".into(),
            },
            Message {
                role: "assistant".into(),
                content: "{}".into(),
            },
        ];
        ensure_condensation_user_query_tail(&mut stack);
        assert_eq!(stack.len(), 3);
        assert_eq!(stack.last().map(|m| m.role.as_str()), Some("user"));
    }

    #[test]
    fn condensation_user_tail_skipped_when_already_user() {
        use super::ensure_condensation_user_query_tail;
        let mut stack = vec![
            Message {
                role: "system".into(),
                content: "instr".into(),
            },
            Message {
                role: "user".into(),
                content: "hi".into(),
            },
        ];
        ensure_condensation_user_query_tail(&mut stack);
        assert_eq!(stack.len(), 2);
    }
}
