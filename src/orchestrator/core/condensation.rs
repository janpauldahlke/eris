use crate::engine::{LlmEngine, Message};
use crate::executive::error::{FcpError, Result};
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

impl<E: LlmEngine> Orchestrator<E> {
    /// Folds older `chat_stack` tail into a rolling JSON summary (sliding window) and retains
    /// recent messages under a token budget. The summary lives only on the chat stack, not in
    /// ephemeral memory.
    pub async fn execute_condensation(&mut self) -> Result<()> {
        if self.chat_stack.is_empty() {
            tracing::warn!("execute_condensation: empty chat stack");
            return Err(FcpError::EngineFault(
                "condensation: empty chat stack".to_string(),
            ));
        }

        let plan = match crate::orchestrator::context::plan_sliding_condensation(
            &self.chat_stack,
            self.num_ctx,
        )? {
            Some(p) => p,
            None => {
                tracing::info!("condensation: nothing to fold; skipping LLM summarizer");
                self.state = AgentState::Chat;
                self.broadcast_state().await;
                return Ok(());
            }
        };

        let instr = crate::orchestrator::context::condensation_system_instruction();
        let summarize_stack = crate::orchestrator::context::build_summarization_stack(
            instr,
            plan.previous_rolling_json.as_deref(),
            &plan.messages_to_fold,
        );

        let retain_budget =
            crate::orchestrator::context::retain_budget_tokens(self.num_ctx).max(32);
        let fold_est = crate::orchestrator::context::estimate_stack_tokens(&plan.messages_to_fold);
        let kept_est = crate::orchestrator::context::estimate_stack_tokens(&plan.kept_tail);
        let summarizer_input_est =
            crate::orchestrator::context::estimate_stack_tokens(&summarize_stack);
        tracing::info!(
            target: "fcp.context_view",
            event = "fcp.condensation.tail_plan",
            num_ctx = self.num_ctx,
            retain_budget_tokens = retain_budget,
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

        let response = self.engine.generate(&summarize_stack, "", None).await?;
        let json_out =
            crate::orchestrator::context::normalize_rolling_summary_response(&response.content)?;

        let mut new_stack = Vec::new();
        new_stack.push(plan.main_system.clone());
        if let Some(jit) = plan.jit.clone() {
            new_stack.push(jit);
        }
        new_stack.push(crate::orchestrator::context::rolling_summary_system_message(&json_out));
        for m in plan.kept_tail {
            new_stack.push(m);
        }
        self.chat_stack = new_stack;

        tracing::info!(
            target: "fcp.context_view",
            event = "fcp.condensation.complete",
            chat_stack_messages_after = self.chat_stack.len(),
            rolling_summary_json_chars = json_out.len(),
            "Condensation complete; rolling summary message replaced on stack"
        );

        self.state = AgentState::Chat;
        self.broadcast_state().await;

        Ok(())
    }
}
