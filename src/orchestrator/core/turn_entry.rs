use crate::engine::LlmEngine;
use crate::executive::error::{FcpError, Result};
use crate::orchestrator::state::AgentState;
use serde_json::json;
use std::collections::HashSet;
use std::time::Instant;

use super::{EMPTY_USER_MESSAGE_TAG, Orchestrator};

const EMPTY_USER_SHRUGS: &[&str] = &["¯\\_(ツ)_/¯", "(・_・)", "(╯°□°）╯︵ ┻━┻"];

impl<E: LlmEngine> Orchestrator<E> {
    /// If the user clearly finished an agenda-linked alarm task, complete it without an LLM round trip.
    pub(super) async fn maybe_run_deterministic_agenda_complete(
        &mut self,
        step_start: Instant,
    ) -> Result<bool> {
        let user_line = self.last_user_content();
        if !Self::user_text_means_agenda_done_ack(user_line) {
            return Ok(false);
        }
        let Some(task_id) = Self::agenda_confirm_task_id_before_current_turn(&self.chat_stack)
        else {
            return Ok(false);
        };

        tracing::info!(
            task_id = %task_id,
            event = "orchestrator.agenda.deterministic_complete",
            "Running agenda:complete from explicit done after AGENDA_CONFIRM"
        );

        let tool_started = Instant::now();
        let args = json!({
            "task_id": task_id,
            "result_summary": "User confirmed completion (deterministic path after agenda alarm)."
        });
        let result = self
            .gatekeeper
            .execute_tool(&AgentState::Idle, "agenda:complete", args)
            .await;
        let tool_ms = tool_started.elapsed().as_millis() as u64;

        match result {
            Ok(tool_out) => {
                let preview = tool_out.chars().take(200).collect::<String>();
                tracing::info!(
                    tool_ms,
                    preview = %preview,
                    event = "orchestrator.agenda.deterministic_complete_ok",
                    "agenda:complete succeeded"
                );
                let deck_msg = format!(
                    "Marked that agenda task as done. {}",
                    tool_out.chars().take(120).collect::<String>()
                );
                let content = serde_json::to_string(&json!({
                    "thought": "User confirmed task completion; agenda:complete executed deterministically.",
                    "status": "Idle",
                    "message_to_user": deck_msg,
                    "tool_calls": []
                }))
                .map_err(|e| FcpError::EngineFault(e.to_string()))?;

                self.emit_optional_user_message(&content).await;
                self.chat_stack.push(crate::engine::Message {
                    role: "assistant".to_string(),
                    content,
                });
                self.state = AgentState::Idle;
                self.recovery_count = 0;
                self.tool_rounds = 0;
                self.last_llm_ms = 0;
                self.last_tool_ms = tool_ms;
                self.last_total_ms = step_start.elapsed().as_millis() as u64;
                self.last_turn_tools_enabled = false;
                self.broadcast_state().await;
                Ok(true)
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    task_id = %task_id,
                    "Deterministic agenda:complete failed; continuing with normal LLM step"
                );
                Ok(false)
            }
        }
    }

    pub(super) async fn handle_empty_user_turn(&mut self) -> Result<()> {
        let idx = self.chat_stack.len() % EMPTY_USER_SHRUGS.len().max(1);
        let face = EMPTY_USER_SHRUGS[idx];
        let thought = format!("{} — empty last user message", EMPTY_USER_MESSAGE_TAG);
        let message_to_user = format!("{face} {}", EMPTY_USER_MESSAGE_TAG);
        let value = serde_json::json!({
            "thought": thought,
            "status": "Idle",
            "message_to_user": message_to_user,
            "tool_calls": []
        });
        let content = serde_json::to_string(&value)?;
        self.emit_optional_user_message(&content).await;
        self.chat_stack.push(crate::engine::Message {
            role: "assistant".to_string(),
            content,
        });
        self.state = AgentState::Idle;
        self.last_llm_ms = 0;
        self.last_total_ms = 0;
        self.broadcast_state().await;
        Ok(())
    }

    pub(super) fn build_descriptor_jit_guidance(
        &self,
        state: &AgentState,
        router_matches: &[String],
        targeted_tools: &HashSet<String>,
    ) -> Option<String> {
        let registry = self.descriptor_registry.as_ref()?;
        let mut selected = if !targeted_tools.is_empty() {
            targeted_tools.iter().cloned().collect::<Vec<_>>()
        } else {
            router_matches
                .iter()
                .take(self.descriptor_jit_top_k.max(1))
                .cloned()
                .collect::<Vec<_>>()
        };
        if selected.is_empty() {
            return None;
        }
        selected.sort();
        selected.dedup();

        let allowed_names = self
            .gatekeeper
            .get_allowed_tools(state)
            .into_iter()
            .filter_map(|t| {
                t.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string())
            })
            .collect::<HashSet<_>>();

        let mut sections = Vec::new();
        let mut used = 0usize;
        let max_chars = self.descriptor_jit_max_chars.max(500);
        for name in selected {
            if !allowed_names.contains(&name) {
                continue;
            }
            let Some(desc) = registry.get(&name) else {
                continue;
            };
            let snippet = format!(
                "Tool: {}\nWhen to use: {}\nWhen not to use: {}\nGood examples: {}\nBad examples: {}",
                desc.tool_name,
                desc.when_to_use.as_deref().unwrap_or("n/a"),
                desc.when_not_to_use.as_deref().unwrap_or("n/a"),
                desc.examples_good
                    .iter()
                    .take(2)
                    .map(|e| format!("{} {}", e.name, e.args))
                    .collect::<Vec<_>>()
                    .join(" | "),
                desc.examples_bad
                    .iter()
                    .take(2)
                    .map(|e| format!("{} {}", e.name, e.args))
                    .collect::<Vec<_>>()
                    .join(" | ")
            );
            if used + snippet.len() > max_chars {
                break;
            }
            used += snippet.len();
            sections.push(snippet);
        }
        if sections.is_empty() {
            return None;
        }
        tracing::debug!(
            jit_section_chars = used,
            jit_section_cap = max_chars,
            selected_tools = sections.len(),
            "Descriptor JIT guidance budget usage"
        );
        Some(format!(
            "[JIT TOOL GUIDANCE]\nUse the following targeted tool guidance while keeping args fully compliant with provided JSON schemas.\n{}\n[/JIT TOOL GUIDANCE]",
            sections.join("\n\n")
        ))
    }
}
