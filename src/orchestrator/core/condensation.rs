use crate::engine::LlmEngine;
use crate::executive::error::{FcpError, Result};
use crate::orchestrator::state::AgentState;

use super::Orchestrator;

impl<E: LlmEngine> Orchestrator<E> {
    /// Folds older `chat_stack` tail into a rolling JSON summary (sliding window), persists it to
    /// ephemeral, and retains recent messages under a token budget.
    pub async fn execute_condensation(&mut self) -> Result<()> {
        if self.chat_stack.is_empty() {
            tracing::warn!("execute_condensation: empty chat stack");
            return Err(FcpError::EngineFault(
                "condensation: empty chat stack".to_string(),
            ));
        }

        let ep_prev = self
            .ephemeral
            .get(crate::orchestrator::context::ROLLING_SUMMARY_TITLE)
            .await;

        let plan = match crate::orchestrator::context::plan_sliding_condensation(
            &self.chat_stack,
            self.num_ctx,
            ep_prev,
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

        let response = self.engine.generate(&summarize_stack, "", None).await?;
        let json_out = crate::orchestrator::context::normalize_rolling_summary_response(
            &response.content,
        )?;

        self.ephemeral
            .upsert_by_title(
                crate::orchestrator::context::ROLLING_SUMMARY_TITLE,
                &json_out,
                vec!["context".to_string(), "rolling_summary".to_string()],
                crate::orchestrator::context::ROLLING_SUMMARY_TTL_SECS,
            )
            .await?;

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

        self.state = AgentState::Chat;
        self.broadcast_state().await;

        Ok(())
    }
}
