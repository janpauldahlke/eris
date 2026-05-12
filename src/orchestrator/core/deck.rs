use crate::engine::LlmEngine;
use crate::orchestrator::llm_support::json_envelope::split_leading_json_object;
use crate::orchestrator::state::LlmResponse;
use crate::presentation::SessionEvent;

use super::Orchestrator;

impl<E: LlmEngine> Orchestrator<E> {
    /// Emits an assistant-facing message to TUI when present in the model JSON.
    ///
    /// **Transcript policy:** `IncomingMessage` (main deck) is sent only when `tool_calls` is empty
    /// — i.e. the model is handing control back with a direct reply (typically `Idle`). When the
    /// model still has `tool_calls`, we do **not** put `message_to_user` on the main transcript:
    /// the same user turn often runs several LLM hops, and an early “I’ve saved it…” line followed
    /// by a correction reads like double answers. For those hops, `message_to_user` is folded into
    /// the orchestrator `activity_line` (tools strip / status) until the final hop. `thought` is always emitted when non-empty.
    pub(super) async fn emit_optional_user_message(&mut self, response_content: &str) {
        let Some(tx) = &self.presentation_tx else {
            return;
        };

        let json_slice = split_leading_json_object(response_content).0;
        let Ok(parsed) = serde_json::from_str::<LlmResponse>(json_slice) else {
            return;
        };

        let thought_trimmed = parsed.thought.trim();
        if !thought_trimmed.is_empty() {
            let thought_len = thought_trimmed.len();
            let preview: String = thought_trimmed.chars().take(120).collect();
            tracing::info!(
                event = "UI_EMIT_MODEL_THOUGHT",
                thought_len,
                preview = %preview,
                "Emitting JSON protocol `thought` to presentation (thought pane / telemetry)"
            );
            if tx
                .send(SessionEvent::ModelThought(thought_trimmed.to_string()))
                .await
                .is_err()
            {
                tracing::warn!(
                    event = "UI_EMIT_MODEL_THOUGHT_DROPPED",
                    thought_len,
                    "Presentation channel closed; model thought not delivered"
                );
            }
        }

        let has_tools = !parsed.tool_calls.is_empty();
        let msg_opt = parsed
            .message_to_user
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        if has_tools {
            let joined = parsed
                .tool_calls
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            tracing::debug!(
                event = "UI_TOOL_ROUND_STATUS",
                tool_count = parsed.tool_calls.len(),
                tool_names = %joined,
                "Tool round: main transcript quiet; message_to_user folded into activity_line until tool_calls empty"
            );
            let tool_names_line = format!("Tools: {}", Self::trim_chars(&joined, 88));
            let activity_line = match &msg_opt {
                Some(msg) => format!(
                    "{}\n{}",
                    tool_names_line,
                    Self::trim_chars(msg.as_str(), 320)
                ),
                None => tool_names_line,
            };
            self.activity_line = Some(activity_line);
            self.broadcast_state().await;
            return;
        }

        let Some(msg) = msg_opt else {
            self.activity_line = None;
            self.broadcast_state().await;
            return;
        };

        self.activity_line = None;
        if self
            .last_deck_message_body
            .as_deref()
            .is_some_and(|prev| prev == msg.as_str())
        {
            tracing::debug!(
                event = "UI_SKIP_DUPLICATE_DECK_MESSAGE",
                msg_len = msg.len(),
                preview = %msg.chars().take(120).collect::<String>(),
                "Skipping duplicate assistant deck message (same body as previous emit this step)"
            );
            self.broadcast_state().await;
            return;
        }
        self.last_deck_message_body = Some(msg.clone());
        let agent_name = self.agent_name();
        tracing::info!(
            event = "UI_EMIT_INCOMING_MESSAGE",
            agent = %agent_name,
            msg_len = msg.len(),
            preview = %msg.chars().take(120).collect::<String>(),
            "Emitting assistant message to TUI deck"
        );
        let _ = tx
            .send(SessionEvent::IncomingMessage(format!(
                "[{}]: {}",
                agent_name, msg
            )))
            .await;
        self.broadcast_state().await;
    }

    /// Deck line for cap-recovery when the normal JSON → deck path did not apply.
    pub(super) async fn emit_assistant_deck_line(&mut self, msg: &str) {
        let Some(tx) = &self.presentation_tx else {
            return;
        };
        let agent_name = self.agent_name();
        let _ = tx
            .send(SessionEvent::IncomingMessage(format!(
                "[{}]: {}",
                agent_name, msg
            )))
            .await;
        self.last_deck_message_body = Some(msg.to_string());
        self.activity_line = None;
        self.broadcast_state().await;
    }
}
