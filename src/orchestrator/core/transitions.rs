use crate::engine::LlmEngine;
use crate::executive::error::Result;
use crate::orchestrator::llm_support::json_envelope::split_leading_json_object;
use crate::orchestrator::r#loop::transition::{StateTransition, TransitionControl};
use crate::orchestrator::state::{AgentState, LlmResponse};
use crate::presentation::SessionEvent;

use super::{Orchestrator, TOOL_ROUND_CAP_USER_FOOTNOTE};

impl<E: LlmEngine> Orchestrator<E> {
    /// Single mutation funnel for state-machine transitions.
    ///
    /// Any transition that changes visible runtime state should be applied
    /// through this method so broadcast/counter behavior stays uniform.
    pub(super) async fn apply_transition(
        &mut self,
        transition: StateTransition,
    ) -> Result<TransitionControl> {
        match transition {
            StateTransition::ExecuteTools(_) => Ok(TransitionControl::ContinueLoop),
            StateTransition::Halt => {
                let user_line = self.last_user_content().to_string();
                crate::memory::turn_end::apply_user_turn_mentions(
                    &*self.ephemeral,
                    &user_line,
                    &self.config,
                )
                .await;
                self.state = AgentState::Idle;
                self.tool_rounds = 0;
                self.recovery_count = 0;
                self.broadcast_state().await;
                Ok(TransitionControl::ReturnOk)
            }
            StateTransition::Recover { message, schema_retry } => {
                self.recovery_count = self.recovery_count.saturating_add(1);
                self.state = AgentState::Recover;
                if schema_retry {
                    tracing::warn!(
                        recovery_count = self.recovery_count,
                        "Schema retry recovery transition"
                    );
                } else {
                    tracing::warn!(recovery_count = self.recovery_count, "Recover transition");
                }
                self.chat_stack.push(crate::engine::Message {
                    role: "system".to_string(),
                    content: message.clone(),
                });
                if let Some(tx) = &self.presentation_tx {
                    let _ = tx
                        .send(SessionEvent::SystemError(message))
                        .await;
                }
                self.broadcast_state().await;
                Ok(TransitionControl::ContinueLoop)
            }
            StateTransition::ShiftToReflection => {
                tracing::info!("Shifting to Reflect state");
                self.state = AgentState::Reflect;
                self.broadcast_state().await;
                Ok(TransitionControl::ContinueLoop)
            }
            StateTransition::Fatal(err) => {
                tracing::error!(error = %err, "Fatal transition applied");
                self.state = AgentState::Idle;
                self.broadcast_state().await;
                Ok(TransitionControl::ContinueLoop)
            }
            StateTransition::Continue => Ok(TransitionControl::ContinueLoop),
        }
    }

    /// After tool-round cap recovery, never run more tools or spin another Reflect hop in the same step.
    pub(super) async fn clamp_transition_for_tool_round_cap_recovery(
        &mut self,
        transition: StateTransition,
        response_content: &str,
    ) -> StateTransition {
        match transition {
            StateTransition::ExecuteTools(_) => {
                tracing::warn!(
                    event = "orchestrator.tool_round_cap.tools_ignored",
                    "Model requested tools after per-turn tool budget was exhausted; idling"
                );
                let json_slice = split_leading_json_object(response_content).0;
                let prefix = serde_json::from_str::<LlmResponse>(json_slice)
                    .ok()
                    .and_then(|p| {
                        p.message_to_user
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                    });
                let body = match prefix {
                    Some(m) => format!("{m}\n\n{TOOL_ROUND_CAP_USER_FOOTNOTE}"),
                    None => TOOL_ROUND_CAP_USER_FOOTNOTE.to_string(),
                };
                self.emit_assistant_deck_line(&body).await;
                StateTransition::Halt
            }
            StateTransition::ShiftToReflection => {
                let json_slice = split_leading_json_object(response_content).0;
                let has_deck = serde_json::from_str::<LlmResponse>(json_slice)
                    .ok()
                    .and_then(|p| {
                        p.message_to_user
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                    })
                    .is_some();
                if !has_deck {
                    self.emit_assistant_deck_line(TOOL_ROUND_CAP_USER_FOOTNOTE).await;
                }
                StateTransition::Halt
            }
            other => other,
        }
    }
}
