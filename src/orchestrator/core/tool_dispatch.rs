use crate::engine::LlmEngine;
use crate::executive::error::{FcpError, Result};
use crate::orchestrator::llm_support::post_tool_guidance::{
    recover_override_message_for_tool_failure, POST_TOOL_STAGED_BUFFER_GUIDANCE,
    POST_TOOL_USER_REPLY_GUIDANCE,
};
use crate::orchestrator::r#loop::recovery_policy::{classify_tool_failure, ToolFailureAction};
use crate::orchestrator::r#loop::tool_batch::ToolBatchDecision;
use crate::orchestrator::state::{AgentState, ToolIntentStatus, ToolIntentTicket};
use crate::presentation::SessionEvent;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

use super::Orchestrator;

impl<E: LlmEngine> Orchestrator<E> {
    /// Read-only buffer tools may repeat with identical args in one user turn (e.g. after condensation
    /// or when the model re-checks the same search).
    fn idempotent_read_tool_allows_repeat(tool_name: &str) -> bool {
        matches!(
            tool_name,
            "ephemeral:buffer_query" | "ephemeral:buffer_page"
        )
    }

    #[allow(clippy::too_many_arguments)]
    /// Executes one tool batch and returns a decision for the coordinator.
    ///
    /// This method owns tool dispatch mechanics; caller applies resulting
    /// transitions via `apply_transition`.
    pub(super) async fn execute_tool_batch(
        &mut self,
        tools: Vec<crate::orchestrator::state::ToolCall>,
        tools_needed: bool,
        execution_ledger: &mut HashMap<String, ToolIntentTicket>,
        schema_recovery_attempted: &mut HashSet<String>,
        targeted_tools: &mut HashSet<String>,
        web_tool_activity: &mut bool,
        buffer_tool_activity: &mut bool,
        tool_ms_acc: &mut u64,
    ) -> Result<ToolBatchDecision> {
        if !tools_needed {
            tracing::info!(
                tool_count = tools.len(),
                "Latent tool intent detected in conversational path"
            );
        }
        tracing::info!(
            event = "orchestrator.tools.batch",
            tool_count = tools.len(),
            "Executing tool calls"
        );
        let current_state = self.state;
        let mut recoverable_msg: Option<String> = None;
        let mut fatal_error = None;
        let mut targeted_recovery_requested = false;
        let mut inject_staged_buffer_followup_hint = false;
        let mut executed_success_count = 0usize;
        let mut suppressed_duplicate_count = 0usize;
        let mut recoverable_fail_count = 0usize;
        let mut fatal_fail_count = 0usize;

        for tool_call in tools {
            let tool_name = tool_call.name;
            let args = tool_call.args;
            let intent_id = Self::tool_fingerprint(&tool_name, &args);
            let suppress_duplicate = execution_ledger
                .get(&intent_id)
                .map(|existing| match existing.status {
                    ToolIntentStatus::Pending => true,
                    ToolIntentStatus::Success => {
                        !Self::idempotent_read_tool_allows_repeat(&tool_name)
                    }
                    ToolIntentStatus::FailedRecoverable | ToolIntentStatus::FailedFatal => false,
                })
                .unwrap_or(false);
            if suppress_duplicate {
                tracing::warn!(
                    tool = %tool_name,
                    intent_id = %intent_id,
                    "Duplicate tool call suppressed in current turn"
                );
                suppressed_duplicate_count += 1;
                let msg = format!(
                    "[SYSTEM] Duplicate tool call suppressed for '{}'. Continue without repeating it.",
                    tool_name
                );
                self.chat_stack.push(crate::engine::Message {
                    role: "system".to_string(),
                    content: msg.clone(),
                });
                if let Some(tx) = &self.presentation_tx {
                    let telemetry = format!("[tool] {} · duplicate suppressed", tool_name);
                    let _ = tx
                        .send(SessionEvent::SystemError(telemetry))
                        .await;
                }
                continue;
            }
            let prev_attempts = execution_ledger
                .get(&intent_id)
                .map(|t| t.attempt_count)
                .unwrap_or(0);
            execution_ledger.insert(
                intent_id.clone(),
                ToolIntentTicket {
                    intent_id: intent_id.clone(),
                    tool_name: tool_name.clone(),
                    args: args.clone(),
                    status: ToolIntentStatus::Pending,
                    attempt_count: prev_attempts.saturating_add(1),
                    last_error: None,
                },
            );
            tracing::debug!(
                tool = %tool_name,
                intent_id = %intent_id,
                "Intent ticket set to Pending"
            );
            tracing::info!(
                tool = %tool_name,
                args = %args,
                state = ?current_state,
                "Dispatching tool"
            );
            let tool_started = Instant::now();
            let result = self
                .gatekeeper
                .execute_tool(&current_state, &tool_name, args.clone())
                .await;
            *tool_ms_acc =
                (*tool_ms_acc).saturating_add(tool_started.elapsed().as_millis() as u64);
            match result {
                Ok(result) => {
                    if let Some(ticket) = execution_ledger.get_mut(&intent_id) {
                        ticket.status = ToolIntentStatus::Success;
                        ticket.last_error = None;
                    }
                    executed_success_count += 1;
                    if tool_name.starts_with("web:") {
                        *web_tool_activity = true;
                    }
                    if matches!(
                        tool_name.as_str(),
                        "ephemeral:buffer_query" | "ephemeral:buffer_page"
                    ) || (tool_name == "vault:read"
                        && (result.contains("lens applied")
                            || result.contains("Large vault file staged as ephemeral buffer")))
                    {
                        *buffer_tool_activity = true;
                    }
                    self.tool_rounds += 1;
                    self.recovery_count = 0;
                    tracing::info!(
                        tool = %tool_name,
                        intent_id = %intent_id,
                        result_len = result.len(),
                        round = self.tool_rounds,
                        "Tool succeeded"
                    );
                    if tool_name == "vault:read"
                        && (result.contains("lens applied")
                            || result.contains("Large vault file staged as ephemeral buffer"))
                    {
                        inject_staged_buffer_followup_hint = true;
                    }
                    if tool_name == "web:fetch" {
                        let t = result.trim();
                        if t.starts_with('{')
                            && let Ok(v) = serde_json::from_str::<serde_json::Value>(t)
                            && v.get("artifact_id").and_then(|a| a.as_str()).is_some()
                            && v.get("chunk_count").and_then(|c| c.as_u64()).unwrap_or(0) > 1
                        {
                            inject_staged_buffer_followup_hint = true;
                        }
                    }
                    let bounded_result = Self::trim_chars(&result, Self::MAX_TOOL_RESULT_CHARS);
                    let msg = crate::orchestrator::context::format_tool_success_line(
                        &tool_name,
                        &bounded_result,
                    );
                    self.chat_stack.push(crate::engine::Message {
                        role: "system".to_string(),
                        content: msg.clone(),
                    });
                    if let Some(tx) = &self.presentation_tx {
                        let telemetry = format!("[tool] {} · success", tool_name);
                        let _ = tx
                            .send(SessionEvent::SystemError(telemetry))
                            .await;
                    }
                    self.broadcast_state().await;
                }
                Err(e) => {
                    tracing::error!(
                        tool = %tool_name,
                        intent_id = %intent_id,
                        error = %e,
                        error_type = ?std::mem::discriminant(&e),
                        "Tool execution failed"
                    );
                    if let Some(ticket) = execution_ledger.get_mut(&intent_id) {
                        ticket.last_error = Some(e.to_string());
                    }
                    let failure_action = classify_tool_failure(
                        &e,
                        schema_recovery_attempted.contains(&tool_name),
                    );
                    match failure_action {
                        ToolFailureAction::TargetedSchemaRetry => {
                            schema_recovery_attempted.insert(tool_name.clone());
                            targeted_tools.insert(tool_name.clone());
                            targeted_recovery_requested = true;
                            recoverable_fail_count += 1;
                            if let Some(ticket) = execution_ledger.get_mut(&intent_id) {
                                ticket.status = ToolIntentStatus::FailedRecoverable;
                            }
                            if recoverable_msg.is_none() {
                                recoverable_msg = Some(e.to_string());
                            }
                            tracing::warn!(tool = %tool_name, "Schema-fault recovery armed for tool");
                        }
                        ToolFailureAction::Recoverable => {
                            recoverable_fail_count += 1;
                            if let Some(ticket) = execution_ledger.get_mut(&intent_id) {
                                ticket.status = ToolIntentStatus::FailedRecoverable;
                            }
                            if recoverable_msg.is_none() {
                                recoverable_msg = Some(e.to_string());
                            }
                        }
                        ToolFailureAction::Fatal => {
                            fatal_fail_count += 1;
                            if let Some(ticket) = execution_ledger.get_mut(&intent_id) {
                                ticket.status = ToolIntentStatus::FailedFatal;
                            }
                            tracing::error!(error = %e, "System fatality detected during tool execution");
                            if fatal_error.is_none() {
                                fatal_error = Some(e);
                            }
                        }
                    }
                }
            }
        }

        let pending_count = execution_ledger
            .values()
            .filter(|t| matches!(t.status, ToolIntentStatus::Pending))
            .count();
        if pending_count > 0 {
            tracing::error!(pending_count, "Pending-state closure invariant violated");
            self.state = AgentState::Idle;
            self.broadcast_state().await;
            return Err(FcpError::EngineFault(format!(
                "Tool intent ledger invariant violated: {pending_count} pending intents after dispatch",
            )));
        }

        tracing::info!(
            event = "orchestrator.tools.batch.summary",
            executed_success_count,
            suppressed_duplicate_count,
            recoverable_fail_count,
            fatal_fail_count,
            "Tool batch outcome summary"
        );

        if executed_success_count == 0
            && recoverable_fail_count == 0
            && fatal_fail_count == 0
            && suppressed_duplicate_count > 0
        {
            tracing::info!("All tool intents in batch were duplicate-suppressed; forcing user-facing reply via recover");
            let msg = "[SYSTEM OVERRIDE] All requested tool calls in this batch were suppressed as duplicates (already executed earlier). Do NOT repeat those tool calls again. Respond to the user now with status Idle and a non-empty message_to_user confirming the outcome. tool_calls MUST be [].".to_string();
            // IMPORTANT: route through Recover so retry is bounded by `max_recovery_attempts`.
            return Ok(ToolBatchDecision::Recover { message: msg });
        }

        if targeted_recovery_requested {
            self.force_full_tool_schemas_in_llm_view = true;
            let selected = targeted_tools.iter().cloned().collect::<Vec<_>>();
            let msg = format!(
                "[SYSTEM RECOVERY] Tool schema fault detected. Retrying with targeted schemas for: {:?}",
                selected
            );
            tracing::info!(targeted_tools = ?selected, "Triggering targeted schema-fault recovery retry");
            return Ok(ToolBatchDecision::RetryWithTargetedSchema { message: msg });
        }

        if let Some(e) = fatal_error {
            return Ok(ToolBatchDecision::Fatal(e));
        }

        if let Some(reason) = recoverable_msg {
            let msg = recover_override_message_for_tool_failure(&reason);
            return Ok(ToolBatchDecision::Recover { message: msg });
        }

        if executed_success_count > 0 {
            self.force_full_tool_schemas_in_llm_view = false;
            // Schema-fault recovery arms `targeted_tools` for one retry with full schemas. If we do
            // not clear it after success, the next loop iteration still uses
            // `assemble_with_selected_tools` for that tool only — the model keeps calling it
            // (e.g. repeated `mail:write`) until it finally returns Idle. See orchestrator field
            // doc on `force_full_tool_schemas_in_llm_view` (same intended lifetime as this set).
            targeted_tools.clear();
            let mut post_tool_block = POST_TOOL_USER_REPLY_GUIDANCE.to_string();
            if inject_staged_buffer_followup_hint {
                post_tool_block.push_str("\n\n");
                post_tool_block.push_str(POST_TOOL_STAGED_BUFFER_GUIDANCE);
            }
            self.chat_stack.push(crate::engine::Message {
                role: "system".to_string(),
                content: post_tool_block,
            });
            tracing::debug!(
                target: "fcp.context_view",
                event = "post_tool_user_reply_guidance_injected",
                staged_buffer_hint = inject_staged_buffer_followup_hint,
                "Post-tool guidance appended after successful tool batch"
            );
        }

        Ok(ToolBatchDecision::Continue)
    }
}
