use crate::engine::LlmEngine;
use crate::orchestrator::llm_support::json_envelope::{
    llm_json_parse_recovery_message_with_excerpt, parse_llm_response_protocol,
};
use crate::orchestrator::state::{
    AgentState, LoopAction, LoopDirective, LlmResponse,
};

use super::Orchestrator;

impl<E: LlmEngine> Orchestrator<E> {
    pub fn process_llm_response(&mut self, response_json: &str) -> LoopDirective {
        match parse_llm_response_protocol(response_json) {
            Ok(parsed) => self.directive_from_parsed(parsed),
            Err(e) => LoopDirective::RecoverFromFuckup(
                llm_json_parse_recovery_message_with_excerpt(&e, response_json),
            ),
        }
    }

    /// Directive path for an already-parsed [`LlmResponse`] (avoids a second parse after `step` preflight).
    pub(super) fn directive_from_parsed(&mut self, parsed: LlmResponse) -> LoopDirective {
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
