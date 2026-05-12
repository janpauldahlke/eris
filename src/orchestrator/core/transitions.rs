use crate::engine::LlmEngine;
use crate::executive::error::Result;
use crate::orchestrator::llm_support::json_envelope::{
    FCP_JSON_REPAIR_MARKER, JSON_REPAIR_UI_SUMMARY, split_leading_json_object,
};
use crate::orchestrator::r#loop::transition::{StateTransition, TransitionControl};
use crate::orchestrator::state::{AgentState, LlmResponse};
use crate::presentation::SessionEvent;

use super::{Orchestrator, TOOL_ROUND_CAP_USER_FOOTNOTE};

/// Max chars for operator-facing `SystemError` lines on schema/tool recover (full text stays on `chat_stack` + tracing).
const RECOVER_UI_TELEMETRY_MAX_CHARS: usize = 360;

fn presentation_recover_ui_summary(message: &str, schema_retry: bool) -> String {
    if message.contains(FCP_JSON_REPAIR_MARKER) {
        return JSON_REPAIR_UI_SUMMARY.to_string();
    }
    if schema_retry {
        return schema_retry_telemetry_summary(message);
    }
    truncate_ui_recover_message(message, RECOVER_UI_TELEMETRY_MAX_CHARS)
}

/// One-line telemetry for targeted schema retry: full NL schema blocks stay in logs / LLM stack only.
fn schema_retry_telemetry_summary(message: &str) -> String {
    let mut tools: Vec<String> = Vec::new();
    for line in message.lines() {
        let t = line.trim();
        let prefix = "Tool \"";
        if let Some(suffix) = t.strip_prefix(prefix) {
            if let Some((name, _)) = suffix.split_once('"') {
                if !name.is_empty() && !tools.iter().any(|x| x == name) {
                    tools.push(name.to_string());
                }
                if tools.len() >= 6 {
                    break;
                }
            }
        }
    }
    if tools.is_empty() {
        "[SYSTEM] Recovery — argument validation failed; retrying with expanded schemas (see core log)."
            .to_string()
    } else {
        format!(
            "[SYSTEM] Recovery — retrying {} with expanded argument schemas (see core log).",
            tools.join(", ")
        )
    }
}

fn truncate_ui_recover_message(message: &str, max_chars: usize) -> String {
    let count = message.chars().count();
    if count <= max_chars {
        return message.to_string();
    }
    let take = max_chars.saturating_sub(24);
    let mut s: String = message.chars().take(take).collect();
    s.push_str("… (see core log)");
    s
}

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
            StateTransition::Recover {
                message,
                schema_retry,
            } => {
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
                    let presentation_line = presentation_recover_ui_summary(&message, schema_retry);
                    let _ = tx.send(SessionEvent::SystemError(presentation_line)).await;
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
                    self.emit_assistant_deck_line(TOOL_ROUND_CAP_USER_FOOTNOTE)
                        .await;
                }
                StateTransition::Halt
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod recover_ui_summary_tests {
    use super::*;

    #[test]
    fn schema_retry_summary_extracts_tool_names() {
        let msg = concat!(
            "[SYSTEM] Recovery\n\n",
            "Tool \"memory:stage\" rejected your arguments.\n\n",
            "Error: x\n\n",
            "---\n\n",
            "Tool \"vault:read\" rejected your arguments.\n",
        );
        let s = schema_retry_telemetry_summary(msg);
        assert!(s.contains("memory:stage"), "{s}");
        assert!(s.contains("vault:read"), "{s}");
        assert!(s.contains("see core log"), "{s}");
    }

    #[test]
    fn schema_retry_summary_fallback_when_no_tool_lines() {
        let s = schema_retry_telemetry_summary("[SYSTEM] Recovery\n\n(no tool headers here)");
        assert!(s.contains("argument validation failed"), "{s}");
    }

    #[test]
    fn truncate_adds_ellipsis_hint() {
        let body = "x".repeat(400);
        let s = truncate_ui_recover_message(&body, 100);
        assert!(s.ends_with("… (see core log)"), "{s}");
        assert!(s.len() < body.len());
    }

    #[test]
    fn json_repair_marker_maps_to_short_line() {
        let msg = format!("parse err\n\n{FCP_JSON_REPAIR_MARKER}\nhint");
        let s = presentation_recover_ui_summary(&msg, true);
        assert_eq!(s, JSON_REPAIR_UI_SUMMARY);
    }
}
