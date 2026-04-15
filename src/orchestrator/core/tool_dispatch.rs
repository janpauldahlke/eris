use crate::engine::LlmEngine;
use crate::executive::error::{FcpError, Result};
use crate::orchestrator::llm_support::post_tool_guidance::{
    recover_override_message_for_tool_failure, POST_TOOL_USER_REPLY_GUIDANCE,
};
use crate::orchestrator::r#loop::recovery_policy::{classify_tool_failure, ToolFailureAction};
use crate::orchestrator::r#loop::tool_batch::ToolBatchDecision;
use crate::orchestrator::state::{AgentState, ToolCall, ToolIntentStatus, ToolIntentTicket};
use crate::presentation::SessionEvent;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

use super::Orchestrator;

/// Dispatch priority within a single tool batch: `clock:now` must run before
/// `db:find_connections` so the model can anchor RFC3339 `when` on live clock output.
/// Calendar RFC3339 fields are primarily anchored by `[SESSION_REFERENCE_TIME]` injected in the system prompt when calendar (or db) tools are offered.
fn tool_dispatch_order_priority(name: &str) -> u8 {
    match name {
        "clock:now" => 0,
        "db:find_connections" => 2,
        _ => 1,
    }
}

fn stable_prioritize_clock_now_before_db(tools: Vec<ToolCall>) -> Vec<ToolCall> {
    let mut indexed: Vec<(usize, ToolCall)> = tools.into_iter().enumerate().collect();
    let before: Vec<String> = indexed.iter().map(|(_, tc)| tc.name.clone()).collect();
    indexed.sort_by_key(|(orig_idx, tc)| (tool_dispatch_order_priority(&tc.name), *orig_idx));
    let out: Vec<ToolCall> = indexed.into_iter().map(|(_, tc)| tc).collect();
    let after: Vec<String> = out.iter().map(|t| t.name.clone()).collect();
    if before != after {
        tracing::info!(
            event = "orchestrator.tools.batch.reordered",
            before = ?before,
            after = ?after,
            "Stable reorder so clock:now runs before db:find_connections"
        );
    }
    out
}

impl<E: LlmEngine> Orchestrator<E> {
    #[allow(clippy::too_many_arguments)]
    /// Executes one tool batch and returns a decision for the coordinator.
    ///
    /// This method owns tool dispatch mechanics; caller applies resulting
    /// transitions via `apply_transition`.
    pub(super) async fn execute_tool_batch(
        &mut self,
        tools: Vec<ToolCall>,
        tools_needed: bool,
        execution_ledger: &mut HashMap<String, ToolIntentTicket>,
        schema_recovery_attempted: &mut HashSet<String>,
        targeted_tools: &mut HashSet<String>,
        web_tool_activity: &mut bool,
        tool_ms_acc: &mut u64,
    ) -> Result<ToolBatchDecision> {
        if !tools_needed {
            tracing::info!(
                tool_count = tools.len(),
                "Latent tool intent detected in conversational path"
            );
        }
        let tools = stable_prioritize_clock_now_before_db(tools);
        tracing::info!(
            event = "orchestrator.tools.batch",
            tool_count = tools.len(),
            "Executing tool calls"
        );
        let current_state = self.state;
        let mut recoverable_msg: Option<String> = None;
        let mut fatal_error = None;
        let mut targeted_recovery_requested = false;
        let mut executed_success_count = 0usize;
        let mut suppressed_duplicate_count = 0usize;
        let mut recoverable_fail_count = 0usize;
        let mut fatal_fail_count = 0usize;

        for tool_call in tools {
            let tool_name = tool_call.name;
            let args = tool_call.args;
            let intent_id = Self::tool_fingerprint(&tool_name, &args);
            if let Some(existing) = execution_ledger.get(&intent_id)
                && matches!(
                    existing.status,
                    ToolIntentStatus::Pending | ToolIntentStatus::Success
                )
            {
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
                    self.tool_rounds += 1;
                    self.recovery_count = 0;
                    tracing::info!(
                        tool = %tool_name,
                        intent_id = %intent_id,
                        result_len = result.len(),
                        round = self.tool_rounds,
                        "Tool succeeded"
                    );
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
            self.chat_stack.push(crate::engine::Message {
                role: "system".to_string(),
                content: POST_TOOL_USER_REPLY_GUIDANCE.to_string(),
            });
            tracing::debug!(
                target: "fcp.context_view",
                event = "post_tool_user_reply_guidance_injected",
                "Post-tool guidance appended after successful tool batch"
            );
        }

        Ok(ToolBatchDecision::Continue)
    }
}

#[cfg(test)]
mod clock_before_db_tests {
    use super::stable_prioritize_clock_now_before_db;
    use crate::orchestrator::state::ToolCall;
    use serde_json::json;

    fn tc(name: &str) -> ToolCall {
        ToolCall {
            name: name.to_string(),
            args: json!({}),
            id: None,
        }
    }

    #[test]
    fn reorders_db_before_clock_to_clock_first_stable() {
        let tools = vec![tc("db:find_connections"), tc("clock:now")];
        let out = stable_prioritize_clock_now_before_db(tools);
        assert_eq!(out[0].name, "clock:now");
        assert_eq!(out[1].name, "db:find_connections");
    }

    #[test]
    fn preserves_relative_order_among_middle_priority_tools() {
        let tools = vec![
            tc("memory:query"),
            tc("db:find_connections"),
            tc("vault:read"),
            tc("clock:now"),
        ];
        let out = stable_prioritize_clock_now_before_db(tools);
        assert_eq!(
            out.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            vec!["clock:now", "memory:query", "vault:read", "db:find_connections"]
        );
    }

    #[test]
    fn unchanged_when_clock_already_first() {
        let tools = vec![tc("clock:now"), tc("memory:query"), tc("db:find_connections")];
        let out = stable_prioritize_clock_now_before_db(tools);
        assert_eq!(
            out.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            vec!["clock:now", "memory:query", "db:find_connections"]
        );
    }
}
