use crate::engine::LlmEngine;
use crate::orchestrator::llm_support::json_envelope::split_leading_json_object;
use crate::orchestrator::state::{
    AgentState, LoopAction, LoopDirective, LlmResponse,
};

use super::Orchestrator;

impl<E: LlmEngine> Orchestrator<E> {
    pub fn process_llm_response(&mut self, response_json: &str) -> LoopDirective {
        let json_str = split_leading_json_object(response_json).0;

        tracing::debug!(extracted_json_len = json_str.len(), "Parsing LLM JSON response");

        let mut parsed: LlmResponse = match serde_json::from_str(json_str) {
            Ok(res) => res,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    raw_snippet = &json_str[..json_str.len().min(200)],
                    "Failed to parse LLM response as JSON"
                );
                return LoopDirective::RecoverFromFuckup(
                    crate::orchestrator::llm_support::json_envelope::llm_json_parse_recovery_message(&e),
                );
            }
        };
        parsed.normalize_tool_calls();

        let explicit_status = parsed.has_explicit_status();
        let status = parsed.status();
        tracing::info!(
            status = ?status,
            explicit_status,
            thought_len = parsed.thought.len(),
            tool_count = parsed.tool_calls.len(),
            has_message = parsed.message_to_user.is_some(),
            "Parsed LLM response"
        );

        if !explicit_status
            && parsed.tool_calls.is_empty()
            && parsed
                .message_to_user
                .as_ref()
                .is_none_or(|m| m.trim().is_empty())
        {
            return LoopDirective::RecoverFromFuckup(
                "Missing required `status` and no actionable fields (`tool_calls`/`message_to_user`)"
                    .to_string(),
            );
        }

        // Tools take precedence: never drop tool_calls because of Idle/Reflect/Task mismatch.
        if !parsed.tool_calls.is_empty() {
            return LoopDirective::ExecuteTools(parsed.tool_calls);
        }

        let tool_mode_empty_action = self.last_turn_tools_enabled
            && parsed.tool_calls.is_empty()
            && parsed
                .message_to_user
                .as_ref()
                .is_none_or(|m| m.trim().is_empty());

        match status {
            LoopAction::Reflect => {
                if let Some(msg) = parsed.message_to_user
                    && !msg.trim().is_empty()
                {
                    return LoopDirective::HaltAndAwaitInput(Some(msg));
                }
                if tool_mode_empty_action {
                    return LoopDirective::RecoverFromFuckup(
                        "Tool-enabled mode forbids empty action: status Reflect with empty tool_calls and empty message_to_user. Use Reflect with tool_calls, or Idle with non-empty message_to_user.".to_string(),
                    );
                }
                tracing::debug!("Reflect with empty tool_calls — treating as Task");
                self.state = AgentState::Chat;
                LoopDirective::ShiftToReflection
            }
            LoopAction::Idle => match parsed.message_to_user {
                Some(msg) if !msg.trim().is_empty() => LoopDirective::HaltAndAwaitInput(Some(msg)),
                _ => {
                    let thought = parsed.thought.trim();
                    if !thought.is_empty() {
                        LoopDirective::HaltAndAwaitInput(Some(thought.to_string()))
                    } else {
                        LoopDirective::RecoverFromFuckup(
                            "Idle status requires non-empty message_to_user (or non-empty thought as fallback)".to_string(),
                        )
                    }
                }
            },
            LoopAction::Task => {
                if tool_mode_empty_action {
                    return LoopDirective::RecoverFromFuckup(
                        "Tool-enabled mode forbids empty action: status Task with empty tool_calls and empty message_to_user. Use Reflect with tool_calls, or Idle with non-empty message_to_user.".to_string(),
                    );
                }
                self.state = AgentState::Chat;
                LoopDirective::ShiftToReflection
            }
        }
    }
}
