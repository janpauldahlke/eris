use crate::engine::LlmEngine;
use crate::executive::error::{FcpError, Result};
use crate::orchestrator::context::build_llm_view;
use crate::orchestrator::llm_support::json_envelope::{split_leading_json_object, trailing_content_after_valid_llm_json};
use crate::orchestrator::r#loop::directive_policy::decide_transition_from_directive;
use crate::orchestrator::r#loop::tool_batch::ToolBatchDecision;
use crate::orchestrator::r#loop::transition::{StateTransition, TransitionControl};
use crate::orchestrator::state::{AgentState, LoopDirective, ToolIntentTicket};
use crate::telemetry::routing_codes;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

use super::{
    Orchestrator, PromotionSuppressedDuringStep, RECOVERY_BUDGET_EXHAUSTED_DECK_LINE,
    TOOL_ROUND_CAP_SYSTEM_GUIDANCE,
};

impl<E: LlmEngine> Orchestrator<E> {
    /// The main cognitive loop.
    ///
    /// Pre-LLM routing: alarm prefix and short-input guard → conversational; else
    /// semantic Top-K for Tier 1 schemas and full roster in Tier 2 (never
    /// conversational on empty semantic match). Always exactly one LLM
    /// generation per user turn unless interrupted.
    #[allow(clippy::never_loop)]
    pub async fn step(&mut self, _user_input: Option<String>) -> Result<()> {
        let _promotion_hold = PromotionSuppressedDuringStep::arm(
            self.promotion_suppressed_during_step.clone(),
        );
        self.turn_seq = self.turn_seq.saturating_add(1);
        let turn_seq = self.turn_seq;
        // No `info_span!().entered()` here: `EnteredSpan` is not `Send` and `step()` awaits
        // inside `tokio::spawn`. Correlation uses `turn_seq` on every routing event instead.

        let step_start = Instant::now();
        let mut llm_ms_acc = 0u64;
        let mut tool_ms_acc = 0u64;
        self.recovery_count = 0;
        self.tool_rounds = 0;
        self.tool_round_cap_final_pass_pending = false;
        self.force_full_tool_schemas_in_llm_view = false;
        self.activity_line = None;
        self.last_deck_message_body = None;
        let mut web_tool_activity = false;
        tracing::info!(
            turn_seq,
            state = ?self.state,
            chat_stack_len = self.chat_stack.len(),
            "step() entered"
        );
        self.broadcast_state().await;

        if self.last_user_content().trim().is_empty() {
            tracing::info!(
                category = routing_codes::CATEGORY_ROUTING,
                issue = routing_codes::ISSUE_STEP_EMPTY_USER_SY_FNORD,
                outcome = routing_codes::OUTCOME_CONVERSATIONAL,
                turn_seq,
                tools_needed = false,
                router_match_count = 0usize,
                "no user text in last message; SY FNORD synthetic reply"
            );
            return self.handle_empty_user_turn().await;
        }

        if self.maybe_run_deterministic_agenda_complete(step_start).await? {
            return Ok(());
        }

        // ── Pre-LLM semantic routing ─────────────────────────────────
        let (mut tools_needed, pre_llm_matched_tools) = self.run_pre_llm_routing().await;
        let mut execution_ledger: HashMap<String, ToolIntentTicket> = HashMap::new();
        let mut schema_recovery_attempted: HashSet<String> = HashSet::new();
        let mut targeted_tools: HashSet<String> = HashSet::new();

        // ── Tool-enabled loop (full schemas) ─────────────────────────
        loop {
            // 1. Bailout Checks
            if self.recovery_count >= self.max_recovery_attempts {
                tracing::warn!(
                    recovery_count = self.recovery_count,
                    max = self.max_recovery_attempts,
                    "Max recovery attempts reached, bailing out"
                );
                let notice = format!(
                    "[fcp] Recovery budget exhausted ({} of {} recovery passes this turn). The assistant is idle — send a new message or simplify the request.",
                    self.recovery_count,
                    self.max_recovery_attempts,
                );
                self.state = AgentState::Idle;
                self.recovery_count = 0;
                self.tool_rounds = 0;
                self.activity_line = None;
                if let Some(tx) = &self.tui_tx {
                    let _ = tx
                        .send(crate::ui::events::TuiEvent::SystemError(notice))
                        .await;
                }
                if self.tui_tx.is_some() {
                    self.emit_assistant_deck_line(RECOVERY_BUDGET_EXHAUSTED_DECK_LINE)
                        .await;
                } else {
                    self.broadcast_state().await;
                }
                return Ok(());
            }
            if self.tool_rounds >= self.max_tool_rounds {
                if !self.tool_round_cap_final_pass_pending {
                    self.tool_round_cap_final_pass_pending = true;
                    tracing::warn!(
                        event = "orchestrator.tool_round_cap.recovery_armed",
                        tool_rounds = self.tool_rounds,
                        max = self.max_tool_rounds,
                        turn_seq,
                        "Max tool rounds reached; injecting final conversational pass (no tools / no JIT)"
                    );
                    let notice = format!(
                        "[fcp] Per-turn tool budget exhausted ({} successful tool runs; max {}). Forcing one final reply without tools — say **continue** if you need more.",
                        self.tool_rounds,
                        self.max_tool_rounds
                    );
                    if let Some(tx) = &self.tui_tx {
                        let _ = tx
                            .send(crate::ui::events::TuiEvent::SystemError(notice))
                            .await;
                    }
                    let guidance = format!(
                        "{}\n\n(Current turn: {} successful tool executions; configured maximum per user turn is {}.)",
                        TOOL_ROUND_CAP_SYSTEM_GUIDANCE,
                        self.tool_rounds,
                        self.max_tool_rounds
                    );
                    self.chat_stack.push(crate::engine::Message {
                        role: "system".to_string(),
                        content: guidance,
                    });
                    tools_needed = false;
                    targeted_tools.clear();
                    continue;
                }
            }

            // 2. Context Assembly (WITH tool schemas)
            self.last_turn_tools_enabled = tools_needed;
            let slim_assembly = self.slim_tool_prompt
                && tools_needed
                && targeted_tools.is_empty()
                && !self.force_full_tool_schemas_in_llm_view;

            let system_prompt = if !tools_needed {
                self.context_assembler
                    .assemble_conversational(&self.ephemeral)
                    .await?
            } else if !targeted_tools.is_empty() {
                let tool_names = targeted_tools.iter().cloned().collect::<Vec<_>>();
                self.context_assembler
                    .assemble_with_selected_tools(
                        &self.state,
                        &self.ephemeral,
                        &self.gatekeeper,
                        &tool_names,
                    )
                    .await?
            } else if slim_assembly {
                let offered: Vec<String> = if pre_llm_matched_tools.is_empty() {
                    vec![]
                } else {
                    let cap = self.tool_map_offer_cap;
                    if cap == 0 {
                        pre_llm_matched_tools.clone()
                    } else {
                        pre_llm_matched_tools.iter().take(cap).cloned().collect()
                    }
                };
                tracing::info!(
                    event = "fcp.tool_prompt.assembly",
                    mode = "slim_phrase_map",
                    offered_count = offered.len(),
                    router_hit_count = pre_llm_matched_tools.len(),
                    cap = self.tool_map_offer_cap,
                    "Slim tool prompt assembly"
                );
                let descriptors = self.descriptor_registry.as_deref();
                self.context_assembler
                    .assemble_slim_tool_map(
                        &self.state,
                        &self.ephemeral,
                        &self.gatekeeper,
                        descriptors,
                        &offered,
                    )
                    .await?
            } else {
                self.context_assembler
                    .assemble(&self.state, &self.ephemeral, &self.gatekeeper)
                    .await?
            };
            tracing::debug!(prompt_len = system_prompt.len(), "System prompt assembled");
            Self::upsert_system_prompt(&mut self.chat_stack, system_prompt);
            if tools_needed
                && let Some(jit_guidance) = self.build_descriptor_jit_guidance(
                    &self.state,
                    &pre_llm_matched_tools,
                    &targeted_tools,
                )
            {
                self.chat_stack.push(crate::engine::Message {
                    role: "system".to_string(),
                    content: jit_guidance,
                });
            }

            tracing::info!(chat_stack_len = self.chat_stack.len(), "Sending to LLM engine");

            // 3. Engine Generation (build view before select! so the interrupt branch can borrow `self.interrupt_rx`.)
            let view_settings = self.llm_view_settings();
            if self.force_full_tool_schemas_in_llm_view {
                tracing::info!(
                    target: "fcp.context_view",
                    "full tool parameter schemas in LLM view (after schema fault; recovery pass)"
                );
            }
            let view = build_llm_view(&self.chat_stack, &view_settings);

            let response_result = tokio::select! {
                res = async {
                    let llm_started = Instant::now();
                    let out = self.engine.generate(&view, "", None).await;
                    llm_ms_acc = llm_ms_acc.saturating_add(llm_started.elapsed().as_millis() as u64);
                    out
                } => res,
                _ = self.interrupt_rx.changed() => {
                    self.saved_chat_state = Some(self.chat_stack.clone());
                    self.chat_stack.clear();

                    let workspace_root = self.context_assembler.core_dir.parent().unwrap_or(&self.context_assembler.core_dir);
                    let agenda_path = crate::vault_layout::agenda_json(workspace_root);

                    let mut active_task = None;
                    if let Ok(content) = tokio::fs::read_to_string(&agenda_path).await
                        && let Ok(tasks) = serde_json::from_str::<Vec<serde_json::Value>>(&content)
                            && let Some(desc) = tasks.first().and_then(|first| first.get("description")).and_then(|d| d.as_str()) {
                                active_task = Some(desc.to_string());
                            }

                    let prompt = if let Some(task) = active_task {
                        format!("You are operating autonomously. Execute this task: {}. When finished, use agenda:complete.", task)
                    } else {
                        "IDLE_STATE".to_string()
                    };

                    self.chat_stack.push(crate::engine::Message {
                        role: "system".to_string(),
                        content: prompt,
                    });
                    self.state = AgentState::Idle;
                    self.broadcast_state().await;
                    return Err(crate::executive::error::FcpError::Interrupted);
                }
            };

            let response = match response_result {
                Ok(res) => {
                    tracing::info!(prompt_tokens = res.prompt_tokens, generated_tokens = res.generated_tokens, content_len = res.content.len(), "LLM response received");
                    tracing::debug!(raw_content = %res.content, "LLM raw output");
                    res
                }
                Err(e) => {
                    tracing::error!(error = %e, "LLM engine generation failed");
                    self.state = AgentState::Idle;
                    self.broadcast_state().await;
                    return Err(e);
                }
            };

            if trailing_content_after_valid_llm_json(&response.content) {
                let (_, tail) = split_leading_json_object(&response.content);
                let preview: String = tail.trim().chars().take(240).collect();
                tracing::warn!(
                    category = routing_codes::CATEGORY_ROUTING,
                    issue = routing_codes::ISSUE_LLM_TRAILING_AFTER_JSON,
                    outcome = "recover",
                    turn_seq,
                    trail_preview = %preview,
                    "JSON protocol violation: non-whitespace after closing JSON object"
                );
                let directive = LoopDirective::RecoverFromFuckup(
                    "Trailing content after the JSON object. Retry; put all user text in message_to_user only."
                        .to_string(),
                );
                let transition = decide_transition_from_directive(directive);
                let control = self.apply_transition(transition).await?;
                self.last_llm_ms = llm_ms_acc;
                self.last_tool_ms = tool_ms_acc;
                self.last_total_ms = step_start.elapsed().as_millis() as u64;
                self.broadcast_state().await;
                if matches!(control, TransitionControl::ContinueLoop) {
                    continue;
                }
                return Ok(());
            }

            self.emit_optional_user_message(&response.content).await;

            self.chat_stack.push(crate::engine::Message {
                role: "assistant".to_string(),
                content: response.content.clone(),
            });

            let total_tokens = response.generated_tokens + response.prompt_tokens;
            let active_threshold_ratio = if web_tool_activity {
                Self::WEB_CONDENSATION_THRESHOLD
            } else {
                self.condensation_threshold
            };
            let threshold = (self.num_ctx as f32 * active_threshold_ratio) as usize;
            if total_tokens > threshold {
                tracing::warn!(
                    total_tokens,
                    threshold,
                    active_threshold_ratio,
                    web_tool_activity,
                    "Token usage exceeds condensation threshold, running condenser"
                );
                self.execute_condensation().await?;
                self.state = AgentState::Reflect;
                self.broadcast_state().await;
            }

            // 4. Directive Processing
            let directive = self.process_llm_response(&response.content);
            tracing::info!(directive = ?directive, "Directive from LLM response");
            let tool_cap_final = self.tool_round_cap_final_pass_pending;
            let mut transition = decide_transition_from_directive(directive);
            if tool_cap_final {
                transition = self
                    .clamp_transition_for_tool_round_cap_recovery(transition, &response.content)
                    .await;
            }
            match transition {
                StateTransition::ExecuteTools(tools) => {
                    let decision = self
                        .execute_tool_batch(
                            tools,
                            tools_needed,
                            &mut execution_ledger,
                            &mut schema_recovery_attempted,
                            &mut targeted_tools,
                            &mut web_tool_activity,
                            &mut tool_ms_acc,
                        )
                        .await?;
                    match decision {
                        ToolBatchDecision::Continue => {}
                        ToolBatchDecision::Halt => {
                            let control = self.apply_transition(StateTransition::Halt).await?;
                            if matches!(control, TransitionControl::ReturnOk) {
                                return Ok(());
                            }
                        }
                        ToolBatchDecision::RetryWithTargetedSchema { message } => {
                            tools_needed = true;
                            self.apply_transition(StateTransition::Recover {
                                message,
                                schema_retry: true,
                            })
                            .await?;
                            continue;
                        }
                        ToolBatchDecision::Recover { message } => {
                            self.apply_transition(StateTransition::Recover {
                                message,
                                schema_retry: false,
                            })
                            .await?;
                        }
                        ToolBatchDecision::Fatal(e) => {
                            tracing::error!(error = %e, "System fatality - aborting orchestrator");
                            self.apply_transition(StateTransition::Fatal(FcpError::EngineFault(
                                e.to_string(),
                            )))
                            .await?;
                            return Err(e);
                        }
                    }
                }
                non_tool_transition => {
                    let control = self.apply_transition(non_tool_transition).await?;
                    if matches!(control, TransitionControl::ReturnOk) {
                        return Ok(());
                    }
                }
            }
            self.last_llm_ms = llm_ms_acc;
            self.last_tool_ms = tool_ms_acc;
            self.last_total_ms = step_start.elapsed().as_millis() as u64;
            self.broadcast_state().await;
        }
    }
}
