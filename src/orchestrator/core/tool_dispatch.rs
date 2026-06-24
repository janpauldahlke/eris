use crate::engine::LlmEngine;
use crate::executive::error::{FcpError, Result};
use crate::orchestrator::context::resolved_tool_recovery::SYSTEM_RECOVERY_PREFIX;
use crate::orchestrator::llm_support::json_envelope::natural_language_schema_description;
use crate::orchestrator::llm_support::post_tool_guidance::{
    POST_TOOL_USER_REPLY_GUIDANCE, POST_TOOL_WEATHER_COMMENT_GUIDANCE,
    ensure_web_find_paired_with_fetch_tools, recover_override_message_for_tool_failure,
    user_wants_media_catalog, vision_see_catalog_nudge,
};
use crate::tools::web::ledger::policy::WEB_FIND_BEFORE_REFETCH;
use crate::orchestrator::r#loop::recovery_policy::{ToolFailureAction, classify_tool_failure};
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
        turn_seq: u64,
        moltbook_overlay_latched: bool,
    ) -> Result<ToolBatchDecision> {
        if !tools_needed {
            tracing::info!(
                tool_count = tools.len(),
                "Latent tool intent detected in conversational path"
            );
        }
        let tools = stable_prioritize_clock_now_before_db(tools);
        let batch_includes_catalog = tools.iter().any(|t| t.name == "media:catalog");
        tracing::info!(
            event = "orchestrator.tools.batch",
            tool_count = tools.len(),
            "Executing tool calls"
        );
        let current_state = self.state;
        let mut recoverable_msg: Option<String> = None;
        let mut fatal_error = None;
        let mut targeted_recovery_requested = false;
        let mut schema_retry_rows: Vec<(String, String)> = Vec::new();
        let mut executed_success_count = 0usize;
        let mut suppressed_duplicate_count = 0usize;
        let mut recoverable_fail_count = 0usize;
        let mut recoverable_failed_tools: Vec<String> = Vec::new();
        let mut fatal_fail_count = 0usize;
        let mut suppressed_repeat_failure_streak = 0usize;
        let mut weather_deck_parts: Vec<(String, String)> = Vec::new();
        let mut non_weather_success = false;

        for tool_call in tools {
            let tool_name = tool_call.name;
            let args = tool_call.args;
            let intent_id = Self::tool_fingerprint(&tool_name, &args);
            let repeatable = self.gatekeeper.tool_allows_repeat(&tool_name);
            let repeat_streak_key = format!("{tool_name}\0{intent_id}");
            if repeatable
                && moltbook_overlay_latched
                && self
                    .tool_repeat_failure_streak
                    .get(&repeat_streak_key)
                    .copied()
                    .unwrap_or(0)
                    >= 2
            {
                suppressed_repeat_failure_streak += 1;
                if let Some(ref mut ledger) = self.moltbook_browse_ledger {
                    ledger.record_repeat_failure_suppressed();
                }
                tracing::info!(
                    turn_seq,
                    tool = %tool_name,
                    intent_id = %intent_id,
                    event = "orchestrator.tools.repeat_failure_suppressed",
                    "Repeated identical repeatable tool after consecutive failures; suppressed"
                );
                let msg = format!(
                    "[SYSTEM] Blocked repeated failure for `{tool_name}` with the same arguments in this turn after consecutive failures. Change `post_id` or other args, or pick a different action."
                );
                self.chat_stack.push(crate::engine::Message {
                    role: "system".to_string(),
                    content: msg.clone(),
                });
                if let Some(tx) = &self.presentation_tx {
                    let telemetry =
                        format!("[tool] {tool_name} · repeat-failure streak suppressed");
                    let _ = tx.send(SessionEvent::SystemError(telemetry)).await;
                }
                continue;
            }
            if !repeatable
                && let Some(existing) = execution_ledger.get(&intent_id)
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
                    let _ = tx.send(SessionEvent::SystemError(telemetry)).await;
                }
                continue;
            }
            if matches!(
                tool_name.as_str(),
                "web:fetch" | "web:search" | "news:today"
            ) {
                let cap = self.config.web.max_web_tool_calls_per_turn;
                if self.web_tool_calls_this_turn >= cap {
                    suppressed_duplicate_count += 1;
                    let msg = format!(
                        "[SYSTEM] Web tool cap reached ({cap}/turn). Answer from existing artifacts via web:find or ask the user to continue."
                    );
                    self.chat_stack.push(crate::engine::Message {
                        role: "system".to_string(),
                        content: msg.clone(),
                    });
                    if let Some(tx) = &self.presentation_tx {
                        let _ = tx
                            .send(SessionEvent::SystemError(format!(
                                "[tool] {tool_name} · WEB_TOOL_TURN_CAP"
                            )))
                            .await;
                    }
                    tracing::info!(
                        tool = %tool_name,
                        intent_id = %intent_id,
                        cap,
                        web_tool_calls_this_turn = self.web_tool_calls_this_turn,
                        event = "orchestrator.tools.web_turn_cap_suppressed",
                        "Web tool call suppressed (per-turn cap); not leaving a pending intent"
                    );
                    continue;
                }
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
            if matches!(
                tool_name.as_str(),
                "web:fetch" | "web:search" | "news:today"
            ) {
                self.web_tool_calls_this_turn = self.web_tool_calls_this_turn.saturating_add(1);
            }
            let gatekeeper_state = crate::tools::gatekeeper::Gatekeeper::dispatch_authorization_state(
                &current_state,
                &tool_name,
                self.force_full_tool_schemas_in_llm_view,
            );
            tracing::info!(
                tool = %tool_name,
                args = %args,
                state = ?current_state,
                gatekeeper_state = ?gatekeeper_state,
                schema_retry = self.force_full_tool_schemas_in_llm_view,
                "Dispatching tool"
            );
            let tool_started = Instant::now();
            let result = self
                .gatekeeper
                .execute_tool(&gatekeeper_state, &tool_name, args.clone())
                .await;
            *tool_ms_acc = (*tool_ms_acc).saturating_add(tool_started.elapsed().as_millis() as u64);
            match result {
                Ok(result) => {
                    self.tool_repeat_failure_streak.remove(&repeat_streak_key);
                    if matches!(
                        tool_name.as_str(),
                        "moltbook:home" | "moltbook:search" | "moltbook:feed"
                    ) && self.moltbook_browse_ledger.is_none()
                    {
                        self.moltbook_browse_ledger =
                            Some(super::moltbook_browse_ledger::MoltbookBrowseLedger::new(
                                turn_seq,
                            ));
                        tracing::info!(
                            turn_seq,
                            tool = %tool_name,
                            event = "moltbook.browse.ledger_opened",
                            "Moltbook browse ledger opened after successful browse entrypoint (cycle policy applies only inside this session)"
                        );
                    }
                    if let Some(ref mut ledger) = self.moltbook_browse_ledger {
                        ledger.record_success(&tool_name, &args);
                    }
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
                    let trim_budget = self.tool_success_trim_budget(&tool_name);
                    let bounded_result = Self::trim_chars(&result, trim_budget);
                    let msg = crate::orchestrator::context::format_tool_success_line(
                        &tool_name,
                        &bounded_result,
                    );
                    self.chat_stack.push(crate::engine::Message {
                        role: "system".to_string(),
                        content: msg.clone(),
                    });
                    if tool_name.starts_with("weather:") {
                        if let Some(report) =
                            crate::tools::weather::report::report_from_tool_envelope(&result)
                        {
                            weather_deck_parts.push((tool_name.clone(), report));
                        }
                    } else {
                        non_weather_success = true;
                    }
                    if tool_name == "vision:see"
                        && user_wants_media_catalog(self.last_user_content())
                        && !batch_includes_catalog
                        && !execution_ledger.values().any(|t| {
                            t.tool_name == "media:catalog"
                                && matches!(t.status, ToolIntentStatus::Success)
                        })
                    {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&result) {
                            let rel = v
                                .get("path")
                                .or_else(|| v.get("relative_path"))
                                .and_then(|x| x.as_str());
                            if let (Some(rel), Some(desc)) = (
                                rel,
                                v.get("description").and_then(|x| x.as_str()),
                            ) {
                                self.chat_stack.push(crate::engine::Message {
                                    role: "system".to_string(),
                                    content: vision_see_catalog_nudge(rel, desc),
                                });
                                tracing::debug!(
                                    target: "fcp.context_view",
                                    event = "vision_see_catalog_nudge_injected",
                                    relative_path = %rel,
                                    "Post vision:see catalog nudge for remember intent"
                                );
                            }
                        }
                    }
                    if tool_name == "vision:display" {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&result) {
                            if v.get("display").and_then(|x| x.as_bool()) == Some(true) {
                                if let (Some(rel), Some(url)) = (
                                    v.get("relative_path").and_then(|x| x.as_str()),
                                    v.get("preview_url").and_then(|x| x.as_str()),
                                ) {
                                    let width = v
                                        .get("width")
                                        .and_then(|x| x.as_u64())
                                        .unwrap_or(0) as u32;
                                    let height = v
                                        .get("height")
                                        .and_then(|x| x.as_u64())
                                        .unwrap_or(0) as u32;
                                    if let Some(tx) = &self.presentation_tx {
                                        let _ = tx
                                            .send(SessionEvent::AssistantImage(
                                                crate::presentation::ImageAttachment {
                                                    relative_path: rel.to_string(),
                                                    preview_url: url.to_string(),
                                                    width,
                                                    height,
                                                },
                                            ))
                                            .await;
                                    }
                                }
                            }
                        }
                    }
                    if tool_name == "web:find" {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&result) {
                            if let Some(summary) =
                                v.get("receipt_summary").and_then(|x| x.as_str())
                            {
                                let mut anchor =
                                    format!("[fcp:web:find_anchor] {summary}");
                                if let Some(url) =
                                    v.get("best_match_url").and_then(|x| x.as_str())
                                {
                                    anchor.push_str(&format!(
                                        " Pass this URL to web:fetch when deepening: {url}"
                                    ));
                                }
                                self.chat_stack.push(crate::engine::Message {
                                    role: "system".to_string(),
                                    content: anchor,
                                });
                            }
                        }
                    }
                    if tool_name == "web:fetch" || tool_name == "web:search" {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&result) {
                            if let Some(artifact_id) =
                                v.get("artifact_id").and_then(|x| x.as_str())
                            {
                                let cached = v
                                    .get("cached")
                                    .and_then(|x| x.as_bool())
                                    .unwrap_or(false);
                                let hint = v
                                    .get("next_step_hint")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or("");
                                let mut anchor = format!(
                                    "[fcp:web:fetch_anchor] artifact_id={artifact_id}"
                                );
                                if !hint.is_empty() {
                                    anchor.push_str(&format!(" {hint}"));
                                }
                                if cached {
                                    anchor.push_str(
                                        " Cache hit — use web:find on this artifact_id; do not assume a new query was fetched.",
                                    );
                                } else {
                                    anchor.push_str(
                                        " Use web:find with artifact_id and query to read the vault body before refetching this host.",
                                    );
                                }
                                self.chat_stack.push(crate::engine::Message {
                                    role: "system".to_string(),
                                    content: anchor,
                                });
                            }
                        }
                    }
                    if let Some(tx) = &self.presentation_tx {
                        let telemetry = format!("[tool] {} · success", tool_name);
                        let _ = tx.send(SessionEvent::SystemError(telemetry)).await;
                    }
                    self.broadcast_state().await;
                }
                Err(err) => {
                    self.step_failed_tools.insert(tool_name.clone());
                    tracing::error!(
                        tool = %tool_name,
                        intent_id = %intent_id,
                        error = %err,
                        error_type = ?std::mem::discriminant(&err),
                        "Tool execution failed"
                    );
                    if repeatable && moltbook_overlay_latched {
                        let streak = self
                            .tool_repeat_failure_streak
                            .entry(repeat_streak_key)
                            .or_insert(0);
                        *streak = (*streak).saturating_add(1);
                    }
                    if let Some(ref mut ledger) = self.moltbook_browse_ledger {
                        ledger.record_moltbook_tool_failure(&tool_name, &err);
                    }
                    if let Some(ticket) = execution_ledger.get_mut(&intent_id) {
                        ticket.last_error = Some(err.to_string());
                    }
                    let failure_action = classify_tool_failure(
                        &err,
                        schema_recovery_attempted.contains(&tool_name),
                    );
                    match failure_action {
                        ToolFailureAction::TargetedSchemaRetry => {
                            schema_recovery_attempted.insert(tool_name.clone());
                            targeted_tools.insert(tool_name.clone());
                            targeted_recovery_requested = true;
                            schema_retry_rows.push((tool_name.clone(), err.to_string()));
                            recoverable_fail_count += 1;
                            if let Some(ticket) = execution_ledger.get_mut(&intent_id) {
                                ticket.status = ToolIntentStatus::FailedRecoverable;
                            }
                            if recoverable_msg.is_none() {
                                recoverable_msg = Some(err.to_string());
                            }
                            tracing::warn!(tool = %tool_name, "Schema-fault recovery armed for tool");
                        }
                        ToolFailureAction::Recoverable => {
                            recoverable_fail_count += 1;
                            recoverable_failed_tools.push(tool_name.clone());
                            if let FcpError::PolicyViolation { code, .. } = &err {
                                if code == WEB_FIND_BEFORE_REFETCH {
                                    targeted_tools.insert("web:find".to_string());
                                }
                            }
                            if let Some(ticket) = execution_ledger.get_mut(&intent_id) {
                                ticket.status = ToolIntentStatus::FailedRecoverable;
                            }
                            if recoverable_msg.is_none() {
                                recoverable_msg = Some(err.to_string());
                            }
                        }
                        ToolFailureAction::Fatal => {
                            fatal_fail_count += 1;
                            if let Some(ticket) = execution_ledger.get_mut(&intent_id) {
                                ticket.status = ToolIntentStatus::FailedFatal;
                            }
                            tracing::error!(error = %err, "System fatality detected during tool execution");
                            if fatal_error.is_none() {
                                fatal_error = Some(err);
                            }
                        }
                    }
                }
            }
        }

        let batch_had_tool_activity = executed_success_count > 0
            || recoverable_fail_count > 0
            || fatal_fail_count > 0
            || suppressed_duplicate_count > 0
            || suppressed_repeat_failure_streak > 0;
        if batch_had_tool_activity {
            if let Some(ledger) = self.moltbook_browse_ledger.as_mut() {
                if let Some(nudge) = ledger.missing_invariant_nudge() {
                    self.chat_stack.push(crate::engine::Message {
                        role: "system".to_string(),
                        content: nudge,
                    });
                    tracing::info!(
                        turn_seq,
                        event = "moltbook.cycle.nudge",
                        "Moltbook browse cycle policy nudge injected"
                    );
                }
                let agenda_xor = crate::tools::agenda::remind_at::agenda_xor_normalized_count_for_logs();
                tracing::info!(
                    turn_seq,
                    event = "moltbook.browse.batch_ledger",
                    moltbook_cycle_id = ledger.started_at_turn_seq,
                    comments_opened_unique_post_ids = ledger.comments_unique_post_ids.len(),
                    repeat_failure_suppressions = ledger.repeat_failure_suppressions,
                    agenda_xor_normalized_count = agenda_xor,
                    home_ok = ledger.home_ok,
                    search_ok = ledger.search_ok,
                    feed_ok = ledger.feed_ok,
                    comments_ok = ledger.comments_ok,
                    votes = ledger.votes,
                    memory_stage = ledger.memory_stage,
                    remind_ok = ledger.remind_ok,
                    last_blocker = ?ledger.last_blocker,
                    "Moltbook browse ledger after tool batch (merge-gate / soak telemetry)"
                );
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
            suppressed_repeat_failure_streak,
            recoverable_fail_count,
            fatal_fail_count,
            "Tool batch outcome summary"
        );

        if executed_success_count == 0
            && recoverable_fail_count == 0
            && fatal_fail_count == 0
            && (suppressed_duplicate_count > 0 || suppressed_repeat_failure_streak > 0)
        {
            tracing::info!(
                suppressed_duplicate_count,
                suppressed_repeat_failure_streak,
                "All tool intents in batch were suppressed (duplicates or repeat-failure streak); forcing user-facing reply via recover"
            );
            return Ok(ToolBatchDecision::SuppressOnlyIdlePass {
                message: crate::orchestrator::llm_support::post_tool_guidance::DUPLICATE_SUPPRESS_IDLE_GUIDANCE
                    .to_string(),
            });
        }

        if targeted_recovery_requested {
            self.force_full_tool_schemas_in_llm_view = true;
            let selected = targeted_tools.iter().cloned().collect::<Vec<_>>();
            let msg = if self.config.is_llamacpp() {
                let mut blocks: Vec<String> = Vec::new();
                for (tool_name, err_msg) in &schema_retry_rows {
                    if let Some(rs) = self.gatekeeper.parameters_root_schema_for(tool_name) {
                        blocks.push(natural_language_schema_description(
                            tool_name,
                            &rs,
                            err_msg,
                        ));
                    } else {
                        blocks.push(format!(
                            "Tool \"{tool_name}\" rejected your arguments.\n\nError: {err_msg}\n\nExpected arguments:\n(No parameter schema is registered for this tool name.)\n\nRetry with corrected tool_calls."
                        ));
                    }
                }
                format!(
                    "{SYSTEM_RECOVERY_PREFIX}\n\n{}",
                    blocks.join("\n\n---\n\n")
                )
            } else {
                format!(
                    "{} — tool schema fault detected. Retrying with targeted schemas for: {:?}",
                    crate::orchestrator::context::resolved_tool_recovery::SYSTEM_RECOVERY_PREFIX,
                    selected
                )
            };
            tracing::info!(targeted_tools = ?selected, "Triggering targeted schema-fault recovery retry");
            return Ok(ToolBatchDecision::RetryWithTargetedSchema { message: msg });
        }

        if let Some(e) = fatal_error {
            return Ok(ToolBatchDecision::Fatal(e));
        }

        if let Some(reason) = recoverable_msg {
            for tool_name in recoverable_failed_tools {
                targeted_tools.insert(tool_name);
            }
            let allowed: HashSet<String> = self
                .gatekeeper
                .get_allowed_tools(&AgentState::Chat)
                .into_iter()
                .filter_map(|t| {
                    t.get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str())
                        .map(|s| s.to_string())
                })
                .collect();
            ensure_web_find_paired_with_fetch_tools(targeted_tools, &allowed);
            if !targeted_tools.is_empty() {
                self.force_full_tool_schemas_in_llm_view = true;
                tracing::info!(
                    targeted_tools = ?targeted_tools,
                    "Recoverable tool failure: targeted full schemas for retry"
                );
            }
            let msg = recover_override_message_for_tool_failure(&reason);
            return Ok(ToolBatchDecision::Recover { message: msg });
        }

        if executed_success_count > 0
            && !weather_deck_parts.is_empty()
            && !non_weather_success
            && executed_success_count == weather_deck_parts.len()
        {
            let parts: Vec<(&str, String)> = weather_deck_parts
                .iter()
                .map(|(n, r)| (n.as_str(), r.clone()))
                .collect();
            let message = crate::tools::weather::report::compose_weather_deck_message(&parts);
            self.pending_weather_deck_report = Some(message);
            targeted_tools.clear();
            self.force_full_tool_schemas_in_llm_view = false;
            self.chat_stack.push(crate::engine::Message {
                role: "system".to_string(),
                content: POST_TOOL_WEATHER_COMMENT_GUIDANCE.to_string(),
            });
            tracing::info!(
                event = "orchestrator.weather.comment_then_report",
                report_blocks = weather_deck_parts.len(),
                "Weather report queued; LLM will add a short comment before append"
            );
            return Ok(ToolBatchDecision::Continue);
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
            vec![
                "clock:now",
                "memory:query",
                "vault:read",
                "db:find_connections"
            ]
        );
    }

    #[test]
    fn unchanged_when_clock_already_first() {
        let tools = vec![
            tc("clock:now"),
            tc("memory:query"),
            tc("db:find_connections"),
        ];
        let out = stable_prioritize_clock_now_before_db(tools);
        assert_eq!(
            out.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            vec!["clock:now", "memory:query", "db:find_connections"]
        );
    }
}

#[cfg(test)]
mod repeat_failure_streak_tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::engine::{EngineResponse, LlmEngine, LlmGenerateOptions, Message};
    use crate::executive::error::Result;
    use crate::memory::ephemeral::EphemeralMemory;
    use crate::orchestrator::context::ContextViewSettings;
    use crate::orchestrator::r#loop::tool_batch::ToolBatchDecision;
    use crate::orchestrator::state::{AgentState, ToolCall, ToolIntentStatus};
    use crate::tools::Gatekeeper;
    use crate::tools::traits::Tool;
    use async_trait::async_trait;
    use schemars::JsonSchema;
    use serde::Deserialize;
    use serde_json::json;
    use std::collections::{HashMap, HashSet};
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::sync::mpsc;

    struct StubEngine;

    #[async_trait]
    impl LlmEngine for StubEngine {
        async fn generate(
            &self,
            _stack: &[Message],
            _available_tools_json: &str,
            _stream_tx: Option<mpsc::UnboundedSender<String>>,
            _options: LlmGenerateOptions,
        ) -> Result<EngineResponse> {
            Ok(EngineResponse {
                content: "{}".into(),
                prompt_tokens: 0,
                generated_tokens: 0,
                generation_ms: 0,
            })
        }
    }

    #[derive(JsonSchema, Deserialize)]
    struct EmptyArgs {}

    struct FailAlwaysTool {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Tool for FailAlwaysTool {
        fn name(&self) -> &'static str {
            "fcp_streak_probe"
        }

        fn description(&self) -> &'static str {
            "test-only repeatable failing tool"
        }

        fn parameters_schema(&self) -> schemars::schema::RootSchema {
            schemars::schema_for!(EmptyArgs)
        }

        fn allow_repeat_in_turn(&self) -> bool {
            true
        }

        async fn execute(&self, _args: serde_json::Value) -> Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(crate::executive::error::FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "intentional".into(),
            })
        }
    }

    fn orchestrator_with_probe(calls: Arc<AtomicUsize>) -> Orchestrator<StubEngine> {
        let mut gatekeeper = Gatekeeper::new();
        gatekeeper.register(Arc::new(FailAlwaysTool {
            calls: calls.clone(),
        }));
        let ephemeral = Arc::new(EphemeralMemory::new("ws".into()));
        let vault_root = Path::new("/tmp/vault");
        let (tx, rx) = tokio::sync::watch::channel(());
        Box::leak(Box::new(tx));
        let (id_tx, id_rx) = tokio::sync::watch::channel(Arc::from("id"));
        Box::leak(Box::new(id_tx));
        Orchestrator::new(
            StubEngine,
            gatekeeper,
            ephemeral,
            vault_root,
            "ws",
            3,
            5,
            0.8,
            4096,
            3,
            6000,
            false,
            0,
            rx,
            None,
            None,
            None,
            ContextViewSettings::default(),
            Arc::new(AppConfig::default()),
            id_rx,
            Arc::new(AtomicBool::new(false)),
            None,
            None,
            None,
            None,
        )
    }

    struct OkWebFetchTool;

    #[async_trait]
    impl Tool for OkWebFetchTool {
        fn name(&self) -> &'static str {
            "web:fetch"
        }

        fn description(&self) -> &'static str {
            "test web fetch"
        }

        fn parameters_schema(&self) -> schemars::schema::RootSchema {
            schemars::schema_for!(EmptyArgs)
        }

        async fn execute(&self, _args: serde_json::Value) -> Result<String> {
            Ok(r#"{"artifact_id":"a","mission_id":"m"}"#.into())
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn web_turn_cap_suppressed_does_not_leave_pending_intent() {
        let mut gatekeeper = Gatekeeper::new();
        gatekeeper.register(Arc::new(OkWebFetchTool));
        let ephemeral = Arc::new(EphemeralMemory::new("ws".into()));
        let vault_root = Path::new("/tmp/vault");
        let (tx, rx) = tokio::sync::watch::channel(());
        Box::leak(Box::new(tx));
        let (id_tx, id_rx) = tokio::sync::watch::channel(Arc::from("id"));
        Box::leak(Box::new(id_tx));
        let mut config = AppConfig::default();
        config.web.max_web_tool_calls_per_turn = 2;
        let mut orch = Orchestrator::new(
            StubEngine,
            gatekeeper,
            ephemeral,
            vault_root,
            "ws",
            3,
            5,
            0.8,
            4096,
            3,
            6000,
            false,
            0,
            rx,
            None,
            None,
            None,
            ContextViewSettings::default(),
            Arc::new(config),
            id_rx,
            Arc::new(AtomicBool::new(false)),
            None,
            None,
            None,
            None,
        );
        orch.state = AgentState::Chat;
        orch.web_tool_calls_this_turn = 2;
        let mut ledger = HashMap::new();
        let mut schema = HashSet::new();
        let mut targeted = HashSet::new();
        let mut web = false;
        let mut tool_ms = 0u64;
        let decision = orch
            .execute_tool_batch(
                vec![ToolCall {
                    name: "web:fetch".into(),
                    args: json!({"url": "https://example.com/"}),
                    id: None,
                }],
                true,
                &mut ledger,
                &mut schema,
                &mut targeted,
                &mut web,
                &mut tool_ms,
                1u64,
                false,
            )
            .await
            .expect("batch must not fatal on cap");
        assert!(matches!(decision, ToolBatchDecision::SuppressOnlyIdlePass { .. }));
        assert!(
            !ledger
                .values()
                .any(|t| matches!(t.status, ToolIntentStatus::Pending)),
            "cap suppression must not leave Pending tickets"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn third_identical_repeatable_suppressed_after_two_failures_when_moltbook_latched() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut orch = orchestrator_with_probe(calls.clone());
        orch.state = AgentState::Chat;
        let mut ledger = HashMap::new();
        let mut schema = HashSet::new();
        let mut targeted = HashSet::new();
        let mut web = false;
        let mut tool_ms = 0u64;
        let tc = || ToolCall {
            name: "fcp_streak_probe".into(),
            args: json!({}),
            id: None,
        };
        let tools = vec![tc(), tc(), tc()];
        let decision = orch
            .execute_tool_batch(
                tools,
                true,
                &mut ledger,
                &mut schema,
                &mut targeted,
                &mut web,
                &mut tool_ms,
                1u64,
                true,
            )
            .await
            .expect("batch");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(matches!(decision, ToolBatchDecision::Recover { .. }));
    }
}

#[cfg(test)]
mod targeted_schema_retry_phase5_tests {
    use super::*;
    use crate::config::{AppConfig, LlmBackend};
    use crate::engine::{EngineResponse, LlmEngine, LlmGenerateOptions, Message};
    use crate::executive::error::Result;
    use crate::memory::ephemeral::EphemeralMemory;
    use crate::orchestrator::context::ContextViewSettings;
    use crate::orchestrator::r#loop::tool_batch::ToolBatchDecision;
    use crate::orchestrator::state::{AgentState, ToolCall};
    use crate::tools::Gatekeeper;
    use crate::tools::traits::Tool;
    use async_trait::async_trait;
    use schemars::JsonSchema;
    use serde::Deserialize;
    use serde_json::json;
    use std::collections::{HashMap, HashSet};
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use tokio::sync::{mpsc, watch};

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct TwoFieldArgs {
        title: String,
        count: i32,
    }

    struct SchemaFaultProbeTool;

    #[async_trait]
    impl Tool for SchemaFaultProbeTool {
        fn name(&self) -> &'static str {
            "fcp_schema_fault_nl_probe"
        }

        fn description(&self) -> &'static str {
            "phase-5 schema retry NL vs JSON path probe"
        }

        fn parameters_schema(&self) -> schemars::schema::RootSchema {
            schemars::schema_for!(TwoFieldArgs)
        }

        async fn execute(&self, _args: serde_json::Value) -> Result<String> {
            Ok("ok".into())
        }
    }

    struct StubEngine;

    #[async_trait]
    impl LlmEngine for StubEngine {
        async fn generate(
            &self,
            _stack: &[Message],
            _available_tools_json: &str,
            _stream_tx: Option<mpsc::UnboundedSender<String>>,
            _options: LlmGenerateOptions,
        ) -> Result<EngineResponse> {
            Ok(EngineResponse {
                content: "{}".into(),
                prompt_tokens: 0,
                generated_tokens: 0,
                generation_ms: 0,
            })
        }
    }

    fn orchestrator_for_schema_retry(backend: LlmBackend) -> Orchestrator<StubEngine> {
        let mut gatekeeper = Gatekeeper::new();
        gatekeeper.register(Arc::new(SchemaFaultProbeTool));
        let ephemeral = Arc::new(EphemeralMemory::new("ws".into()));
        let vault_root = Path::new("/tmp/vault");
        let (tx, rx) = watch::channel(());
        Box::leak(Box::new(tx));
        let (id_tx, id_rx) = watch::channel(Arc::from("id"));
        Box::leak(Box::new(id_tx));
        let mut cfg = AppConfig::default();
        cfg.llm_backend = backend;
        Orchestrator::new(
            StubEngine,
            gatekeeper,
            ephemeral,
            vault_root,
            "ws",
            3,
            5,
            0.8,
            4096,
            3,
            6000,
            false,
            0,
            rx,
            None,
            None,
            None,
            ContextViewSettings::default(),
            Arc::new(cfg),
            id_rx,
            Arc::new(AtomicBool::new(false)),
            None,
            None,
            None,
            None,
        )
    }

    #[tokio::test(flavor = "current_thread")]
    async fn targeted_retry_uses_natural_language_for_llamacpp() {
        let mut orch = orchestrator_for_schema_retry(LlmBackend::LlamaCpp);
        orch.state = AgentState::Chat;
        let mut ledger = HashMap::new();
        let mut schema = HashSet::new();
        let mut targeted = HashSet::new();
        let mut web = false;
        let mut tool_ms = 0u64;
        let tools = vec![ToolCall {
            name: "fcp_schema_fault_nl_probe".into(),
            args: json!({}),
            id: None,
        }];
        let decision = orch
            .execute_tool_batch(
                tools,
                true,
                &mut ledger,
                &mut schema,
                &mut targeted,
                &mut web,
                &mut tool_ms,
                1u64,
                false,
            )
            .await
            .expect("batch");
        match decision {
            ToolBatchDecision::RetryWithTargetedSchema { message } => {
                assert!(
                    message.contains("Expected arguments:"),
                    "NL schema recovery: {message}"
                );
                assert!(message.contains("title"));
                assert!(message.contains("count"));
            }
            other => panic!("expected RetryWithTargetedSchema, got {:?}", other),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn targeted_retry_uses_json_schema_for_ollama() {
        let mut orch = orchestrator_for_schema_retry(LlmBackend::Ollama);
        orch.state = AgentState::Chat;
        let mut ledger = HashMap::new();
        let mut schema = HashSet::new();
        let mut targeted = HashSet::new();
        let mut web = false;
        let mut tool_ms = 0u64;
        let tools = vec![ToolCall {
            name: "fcp_schema_fault_nl_probe".into(),
            args: json!({}),
            id: None,
        }];
        let decision = orch
            .execute_tool_batch(
                tools,
                true,
                &mut ledger,
                &mut schema,
                &mut targeted,
                &mut web,
                &mut tool_ms,
                1u64,
                false,
            )
            .await
            .expect("batch");
        match decision {
            ToolBatchDecision::RetryWithTargetedSchema { message } => {
                assert!(message.contains("tool schema fault detected"));
                assert!(
                    !message.contains("Expected arguments:"),
                    "Ollama path should keep legacy short recovery banner: {message}"
                );
            }
            other => panic!("expected RetryWithTargetedSchema, got {:?}", other),
        }
    }
}
