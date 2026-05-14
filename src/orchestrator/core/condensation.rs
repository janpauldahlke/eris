use crate::engine::{LlmEngine, LlmGenerateOptions, Message};
use crate::executive::error::{FcpError, Result};
use crate::orchestrator::context::resolved_tool_recovery::is_recovery_system_content;
use crate::orchestrator::context::{split_stack_head, trim_chat_stack_to_est_token_ceiling};
use crate::orchestrator::state::AgentState;

use super::Orchestrator;

/// Role, length, and a short one-line-ish snippet per message — capped for logs only.
fn condensation_messages_outline(messages: &[Message]) -> String {
    const MAX_MSGS: usize = 12;
    const SNIPPET_CHARS: usize = 56;
    const MAX_OUT_CHARS: usize = 1800;

    let mut parts: Vec<String> = Vec::new();
    for (idx, m) in messages.iter().take(MAX_MSGS).enumerate() {
        let total = m.content.chars().count();
        let snippet: String = m
            .content
            .chars()
            .filter(|&c| !matches!(c, '\n' | '\r' | '\t'))
            .take(SNIPPET_CHARS)
            .collect();
        let suffix = if total > SNIPPET_CHARS { "…" } else { "" };
        parts.push(format!(
            "[{}#{} {}ch] {}{}",
            m.role, idx, total, snippet, suffix
        ));
    }
    if messages.len() > MAX_MSGS {
        parts.push(format!(
            "+{} more msgs",
            messages.len().saturating_sub(MAX_MSGS)
        ));
    }
    let mut joined = parts.join(" | ");
    if joined.chars().count() > MAX_OUT_CHARS {
        joined = joined.chars().take(MAX_OUT_CHARS).collect::<String>();
        joined.push('…');
    }
    joined
}

fn strip_recovery_system_rows_after_head(stack: &mut Vec<Message>) -> usize {
    let Ok(head) = split_stack_head(stack) else {
        return 0;
    };
    let n_head = 1 + usize::from(head.jit.is_some()) + usize::from(head.rolling.is_some());
    if stack.len() <= n_head {
        return 0;
    }
    let mut removed = 0usize;
    let mut i = n_head;
    while i < stack.len() {
        let remove = stack[i].role == "system" && is_recovery_system_content(&stack[i].content);
        if remove {
            stack.remove(i);
            removed = removed.saturating_add(1);
        } else {
            i += 1;
        }
    }
    removed
}

impl<E: LlmEngine> Orchestrator<E> {
    /// Folds older `chat_stack` tail into a rolling JSON summary (sliding window) and retains
    /// recent messages under a token budget. The summary lives only on the chat stack, not in
    /// ephemeral memory.
    ///
    /// May run up to [`crate::config::AppConfig::condensation_max_chained_passes`] summarizer
    /// passes when the stack stays above the configured estimated ceiling, then optionally hard-trims
    /// the tail (still preserving the latest user turn).
    pub async fn execute_condensation(&mut self) -> Result<()> {
        if self.chat_stack.is_empty() {
            tracing::warn!("execute_condensation: empty chat stack");
            return Err(FcpError::EngineFault(
                "condensation: empty chat stack".to_string(),
            ));
        }

        if self.config.condensation_strip_recovery_system_messages {
            let n = strip_recovery_system_rows_after_head(&mut self.chat_stack);
            if n > 0 {
                tracing::info!(
                    target: "fcp.context_view",
                    event = "fcp.condensation.strip_recovery",
                    removed = n,
                    "Removed recovery / JSON-repair system rows before folding"
                );
            }
        }

        let max_passes = self.config.condensation_max_chained_passes.max(1);
        let retain_ratio = self.config.condensation_retain_ratio;
        let ceiling = self.config.condensation_stack_est_ceiling_tokens(self.num_ctx);

        let mut any_fold = false;
        let mut nothing_on_first_plan = false;

        for pass_idx in 0..max_passes {
            let plan = match crate::orchestrator::context::plan_sliding_condensation(
                &self.chat_stack,
                self.num_ctx,
                retain_ratio,
            )? {
                Some(p) => p,
                None => {
                    if pass_idx == 0 {
                        nothing_on_first_plan = true;
                    }
                    break;
                }
            };

            any_fold = true;

            let instr = crate::orchestrator::context::condensation_system_instruction();
            let mut summarize_stack = crate::orchestrator::context::build_summarization_stack(
                instr,
                plan.previous_rolling_json.as_deref(),
                &plan.messages_to_fold,
            );
            crate::orchestrator::context::ensure_condensation_user_query_tail(&mut summarize_stack);

            let retain_budget = crate::orchestrator::context::retain_budget_tokens(
                self.num_ctx,
                retain_ratio,
            )
            .max(32);
            let fold_est =
                crate::orchestrator::context::estimate_stack_tokens(&plan.messages_to_fold);
            let kept_est = crate::orchestrator::context::estimate_stack_tokens(&plan.kept_tail);
            let summarizer_input_est =
                crate::orchestrator::context::estimate_stack_tokens(&summarize_stack);
            tracing::info!(
                target: "fcp.context_view",
                event = "fcp.condensation.tail_plan",
                pass = pass_idx + 1,
                max_passes,
                num_ctx = self.num_ctx,
                retain_budget_tokens = retain_budget,
                stack_est_ceiling_tokens = ceiling,
                chat_stack_messages_before = self.chat_stack.len(),
                fold_message_count = plan.messages_to_fold.len(),
                kept_tail_message_count = plan.kept_tail.len(),
                fold_est_tokens = fold_est,
                kept_tail_est_tokens = kept_est,
                prior_rolling_summary = plan.previous_rolling_json.is_some(),
                summarizer_stack_messages = summarize_stack.len(),
                summarizer_input_est_tokens = summarizer_input_est,
                "Condensation tail plan: fold older messages into rolling summary; keep recent under retain budget"
            );
            tracing::debug!(
                target: "fcp.context_view",
                event = "fcp.condensation.tail_outline",
                fold_outline = %condensation_messages_outline(&plan.messages_to_fold),
                kept_outline = %condensation_messages_outline(&plan.kept_tail),
                "Condensation fold vs kept (bounded role/len/snippets; requires debug log level)"
            );

            let response = self
                .engine
                .generate(
                    &summarize_stack,
                    "",
                    None,
                    LlmGenerateOptions {
                        attach_session_grammar: false,
                        ..Default::default()
                    },
                )
                .await?;
            let json_out =
                crate::orchestrator::context::normalize_rolling_summary_response(&response.content)?;

            let mut new_stack = Vec::new();
            new_stack.push(plan.main_system.clone());
            if let Some(jit) = plan.jit.clone() {
                new_stack.push(jit);
            }
            new_stack.push(crate::orchestrator::context::rolling_summary_system_message(
                &json_out,
            ));
            for m in plan.kept_tail {
                new_stack.push(m);
            }
            self.chat_stack = new_stack;

            tracing::info!(
                target: "fcp.context_view",
                event = "fcp.condensation.complete",
                pass = pass_idx + 1,
                chat_stack_messages_after = self.chat_stack.len(),
                rolling_summary_json_chars = json_out.len(),
                "Condensation pass complete; rolling summary message replaced on stack"
            );

            if pass_idx + 1 >= max_passes {
                break;
            }
            let est = crate::orchestrator::context::estimate_stack_tokens(&self.chat_stack);
            if est <= ceiling {
                break;
            }
        }

        let dropped = if crate::orchestrator::context::estimate_stack_tokens(&self.chat_stack)
            > ceiling
        {
            let d = trim_chat_stack_to_est_token_ceiling(&mut self.chat_stack, ceiling)?;
            if d > 0 {
                tracing::warn!(
                    target: "fcp.context_view",
                    event = "fcp.condensation.hard_trim",
                    dropped_messages = d,
                    ceiling_tokens = ceiling,
                    stack_est_after = crate::orchestrator::context::estimate_stack_tokens(
                        &self.chat_stack
                    ),
                    "Stack still above estimated ceiling after condensation chain; dropped oldest tail messages"
                );
            } else if !any_fold {
                tracing::warn!(
                    target: "fcp.context_view",
                    event = "fcp.condensation.over_ceiling_no_trim",
                    ceiling_tokens = ceiling,
                    stack_est = crate::orchestrator::context::estimate_stack_tokens(&self.chat_stack),
                    "Stack exceeds estimated ceiling but nothing could be folded or trimmed (tail may be one anchored block)"
                );
            }
            d
        } else {
            0
        };

        if nothing_on_first_plan && !any_fold && dropped == 0 {
            tracing::info!("condensation: nothing to fold; skipping LLM summarizer");
        } else {
            tracing::info!(
                target: "fcp.context_view",
                event = "fcp.condensation.session",
                folded = any_fold,
                hard_trimmed = dropped,
                "Condensation session finished"
            );
        }

        self.state = AgentState::Chat;
        self.broadcast_state().await;

        Ok(())
    }
}
