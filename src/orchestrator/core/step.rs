use crate::engine::{LlmEngine, LlmGenerateOptions};
use crate::executive::error::{FcpError, Result};
use crate::orchestrator::context::{build_llm_view, estimate_stack_tokens};
use crate::orchestrator::llm_support::json_envelope::{
    parse_llm_response_protocol, split_leading_json_object, trailing_content_after_valid_llm_json,
};
use crate::orchestrator::r#loop::directive_policy::decide_transition_from_directive;
use crate::orchestrator::r#loop::tool_batch::ToolBatchDecision;
use crate::orchestrator::r#loop::transition::{StateTransition, TransitionControl};
use crate::orchestrator::state::{AgentState, LoopAction, LoopDirective, ToolIntentTicket};
use crate::presentation::SessionEvent;
use crate::telemetry::routing_codes;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

use super::{
    llama_gbnf_subset::slim_offered_tool_names, Orchestrator, PromotionSuppressedDuringStep,
    RECOVERY_BUDGET_EXHAUSTED_DECK_LINE, TOOL_ROUND_CAP_SYSTEM_GUIDANCE,
};
use crate::config::AppConfig;

/// Sampling temperature for LLM calls after at least one recovery message was pushed this `step()`.
const RECOVERY_PASS_LLM_TEMPERATURE: f32 = 0.25;

/// Ollama-only: grammar-constrained backends cannot emit trailing prose after the JSON object.
pub(crate) fn trailing_json_recovery_triggered(config: &AppConfig, content: &str) -> bool {
    !config.is_llamacpp() && trailing_content_after_valid_llm_json(content)
}

impl<E: LlmEngine> Orchestrator<E> {
    /// The main cognitive loop.
    ///
    /// Pre-LLM routing: alarm prefix and short-input guard → conversational; else
    /// semantic Top-K for Tier 1 schemas and full roster in Tier 2 (never
    /// conversational on empty semantic match). Always exactly one LLM
    /// generation per user turn unless interrupted.
    #[allow(clippy::never_loop)]
    pub async fn step(&mut self, _user_input: Option<String>) -> Result<()> {
        let _promotion_hold =
            PromotionSuppressedDuringStep::arm(self.promotion_suppressed_during_step.clone());
        self.turn_seq = self.turn_seq.saturating_add(1);
        let turn_seq = self.turn_seq;
        self.moltbook_browse_ledger = None;
        self.tool_repeat_failure_streak.clear();
        self.step_failed_tools.clear();
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
        self.last_prefetch_ms = 0;
        self.context_assembler.set_turn_prefetch_block(None);
        self.context_assembler.set_turn_document_prefetch_block(None);
        let mut web_tool_activity = false;
        self.web_tool_calls_this_turn = 0;
        if let Some(ledger) = &self.web_ledger {
            ledger.lock().await.begin_user_turn();
        }
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

        if self
            .maybe_run_deterministic_agenda_complete(step_start)
            .await?
        {
            return Ok(());
        }

        // ── Turn-start prefetch (memory + document, in parallel) ──
        {
            let user_text = self.last_user_content().to_string();
            let prefetch_started = Instant::now();

            let memory_fut = async {
                if let Some(semantic) = self.semantic.as_ref() {
                    crate::memory::prefetch::run_turn_prefetch(semantic, &user_text, &self.config).await
                } else {
                    None
                }
            };
            let doc_fut = async {
                if let Some(ds) = self.document_store.as_ref() {
                    crate::memory::prefetch::run_document_prefetch(ds, &user_text, &self.config).await
                } else {
                    None
                }
            };

            let (memory_block, doc_block) = tokio::join!(memory_fut, doc_fut);

            let mut activity_parts: Vec<String> = Vec::new();
            if let Some(block) = memory_block {
                self.last_prefetch_ms = prefetch_started.elapsed().as_millis() as u64;
                let hit_count = block.matches('\n').count().saturating_add(1);
                activity_parts.push(format!("{hit_count} memory fact(s)"));
                self.context_assembler.set_turn_prefetch_block(Some(block));
            }
            if let Some(block) = doc_block {
                let hit_count = block.matches("(from ").count().max(1);
                activity_parts.push(format!("{hit_count} document passage(s)"));
                self.context_assembler.set_turn_document_prefetch_block(Some(block));
            }
            if !activity_parts.is_empty() {
                self.activity_line = Some(format!("Recalled {}", activity_parts.join(" + ")));
            }
        }

        // ── Pre-LLM semantic routing ─────────────────────────────────
        let (mut tools_needed, pre_llm_matched_tools) = self.run_pre_llm_routing().await;
        let mut execution_ledger: HashMap<String, ToolIntentTicket> = HashMap::new();
        let mut schema_recovery_attempted: HashSet<String> = HashSet::new();
        let mut targeted_tools: HashSet<String> = HashSet::new();
        // Once true, keep appending `00_Invariants/Moltbook.md` for every inner LLM round in
        // this `step()`, so later recovery/tool rounds do not drop the overlay after JIT or
        // `targeted_tools` shift mid-loop.
        let mut moltbook_overlay_latched = false;

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
                    self.recovery_count, self.max_recovery_attempts,
                );
                self.state = AgentState::Idle;
                self.recovery_count = 0;
                self.tool_rounds = 0;
                self.activity_line = None;
                if let Some(tx) = &self.presentation_tx {
                    let _ = tx.send(SessionEvent::SystemError(notice)).await;
                }
                if self.presentation_tx.is_some() {
                    self.emit_assistant_deck_line(RECOVERY_BUDGET_EXHAUSTED_DECK_LINE)
                        .await;
                } else {
                    self.broadcast_state().await;
                }
                return Ok(());
            }
            if self.tool_rounds >= self.max_tool_rounds
                && self.state != AgentState::Reflect
            {
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
                        self.tool_rounds, self.max_tool_rounds
                    );
                    if let Some(tx) = &self.presentation_tx {
                        let _ = tx.send(SessionEvent::SystemError(notice)).await;
                    }
                    let guidance = format!(
                        "{}\n\n(Current turn: {} successful tool executions; configured maximum per user turn is {}.)",
                        TOOL_ROUND_CAP_SYSTEM_GUIDANCE, self.tool_rounds, self.max_tool_rounds
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
            let moltbook_overlay_base = self
                .last_user_content()
                .to_ascii_lowercase()
                .contains("moltbook")
                || pre_llm_matched_tools
                    .iter()
                    .any(|name| name.starts_with("moltbook:"))
                || targeted_tools
                    .iter()
                    .any(|name| name.starts_with("moltbook:"));
            if moltbook_overlay_base {
                moltbook_overlay_latched = true;
            }
            let moltbook_overlay = moltbook_overlay_latched;

            let mut system_prompt = if !tools_needed {
                self.context_assembler
                    .assemble_conversational(&self.state, &self.ephemeral, moltbook_overlay)
                    .await?
            } else if !targeted_tools.is_empty() {
                let tool_names = targeted_tools.iter().cloned().collect::<Vec<_>>();
                self.context_assembler
                    .assemble_with_selected_tools(
                        &self.state,
                        &self.ephemeral,
                        &self.gatekeeper,
                        &tool_names,
                        moltbook_overlay,
                    )
                    .await?
            } else if slim_assembly {
                // Single source of truth shared with the GBNF subset path below, so the
                // grammar and the slim tool map can never drift apart.
                let offered = slim_offered_tool_names(
                    &pre_llm_matched_tools,
                    self.tool_map_offer_cap,
                    moltbook_overlay_latched,
                    &self.gatekeeper,
                    &self.state,
                );
                tracing::info!(
                    event = "fcp.tool_prompt.assembly",
                    mode = "slim_phrase_map",
                    offered_count = offered.len(),
                    router_hit_count = pre_llm_matched_tools.len(),
                    cap = self.tool_map_offer_cap,
                    moltbook_overlay_latched,
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
                        moltbook_overlay,
                    )
                    .await?
            } else {
                self.context_assembler
                    .assemble(
                        &self.state,
                        &self.ephemeral,
                        &self.gatekeeper,
                        moltbook_overlay,
                    )
                    .await?
            };
            tracing::debug!(prompt_len = system_prompt.len(), "System prompt assembled");

            // Compute JIT guidance before mutating chat_stack (immutable borrows only).
            let jit_guidance = if tools_needed {
                self.build_descriptor_jit_guidance(
                    &self.state,
                    &pre_llm_matched_tools,
                    &targeted_tools,
                )
            } else {
                None
            };
            let skill_guidance = if tools_needed {
                self.build_skill_jit_guidance(
                    &self.state,
                    &pre_llm_matched_tools,
                    &targeted_tools,
                    &self.step_failed_tools,
                )
                .await?
            } else {
                None
            };

            // Merge JIT/skill guidance into the system prompt so all system
            // content stays at index 0.  Strict chat templates (Qwen / llama.cpp)
            // reject system messages after non-system messages.
            if let Some(jit) = jit_guidance {
                system_prompt.push_str("\n\n---\n\n");
                system_prompt.push_str(&jit);
            }
            if let Some(skill) = skill_guidance {
                system_prompt.push_str("\n\n---\n\n");
                system_prompt.push_str(&skill);
            }
            Self::upsert_system_prompt(&mut self.chat_stack, system_prompt);

            if self.config.optimize_context_proactive_condensation {
                let est = estimate_stack_tokens(&self.chat_stack);
                let active_ratio = if web_tool_activity {
                    Self::WEB_CONDENSATION_THRESHOLD
                } else {
                    self.condensation_threshold
                };
                let ratio = self
                    .config
                    .optimize_context_proactive_condensation_ratio
                    .clamp(0.05, 1.0);
                let threshold_line = (self.num_ctx as f32 * active_ratio * ratio).max(1.0) as usize;
                if est > threshold_line {
                    tracing::info!(
                        target: "fcp.context_view",
                        event = "fcp.condensation.proactive",
                        stack_est_tokens = est,
                        threshold_tokens = threshold_line,
                        proactive_ratio = ratio,
                        active_threshold_ratio = active_ratio,
                        turn_seq,
                        "Estimated stack tokens exceed proactive threshold; folding before main generate"
                    );
                    self.execute_condensation().await?;
                }
            }

            tracing::info!(
                chat_stack_len = self.chat_stack.len(),
                "Sending to LLM engine"
            );

            // 3. Engine Generation (build view before select! so the interrupt branch can borrow `self.interrupt_rx`.)
            let view_settings = self.llm_view_settings();
            if self.force_full_tool_schemas_in_llm_view {
                tracing::info!(
                    target: "fcp.context_view",
                    "full tool parameter schemas in LLM view (after schema fault; recovery pass)"
                );
            }
            let view = build_llm_view(&self.chat_stack, &view_settings);

            // llama.cpp: GBNF must match the tool names visible in this hop's system prompt.
            // - Conversational / tool-cap final pass: no-tool envelope (empty `tool_calls` only).
            // - `assemble_with_selected_tools`: grammar ⊆ `targeted_tools`.
            // - Slim phrase map: same `offered` list as assembly (including Moltbook union).
            //   When `offered` is empty, [`ContextAssembler::assemble_slim_tool_map`] still injects
            //   the full allowed roster (`filter_tools_by_offered_order`); use session GBNF so
            //   grammar and prompt stay consistent (subset grammar would wrongly allow only `[]`).
            // - Full `assemble` (including recovery when `force_full_tool_schemas_in_llm_view`
            //   disables slim): session grammar from `set_grammar` (full registry).
            let (grammar_override, attach_session_grammar) = if !self.config.is_llamacpp() {
                (None, true)
            } else if !tools_needed {
                let g = self
                    .gbnf_subset_cache
                    .get_or_compile_subset(&self.gatekeeper, &[])?;
                (Some(g), true)
            } else if !targeted_tools.is_empty() {
                let names: Vec<String> = targeted_tools.iter().cloned().collect();
                let g = self
                    .gbnf_subset_cache
                    .get_or_compile_subset(&self.gatekeeper, &names)?;
                (Some(g), true)
            } else if slim_assembly {
                let offered = slim_offered_tool_names(
                    &pre_llm_matched_tools,
                    self.tool_map_offer_cap,
                    moltbook_overlay_latched,
                    &self.gatekeeper,
                    &self.state,
                );
                if offered.is_empty() {
                    (None, true)
                } else {
                    let g = self
                        .gbnf_subset_cache
                        .get_or_compile_subset(&self.gatekeeper, &offered)?;
                    (Some(g), true)
                }
            } else {
                (None, true)
            };

            let response_result = tokio::select! {
                res = async {
                    let llm_started = Instant::now();
                    let gen_options = LlmGenerateOptions {
                        temperature: if self.recovery_count > 0 {
                            Some(RECOVERY_PASS_LLM_TEMPERATURE)
                        } else {
                            None
                        },
                        grammar_override,
                        attach_session_grammar,
                    };
                    let out = self.engine.generate(&view, "", None, gen_options).await;
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
                    tracing::info!(
                        prompt_tokens = res.prompt_tokens,
                        generated_tokens = res.generated_tokens,
                        content_len = res.content.len(),
                        "LLM response received"
                    );
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

            if trailing_json_recovery_triggered(&self.config, &response.content) {
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
                if matches!(control, TransitionControl::ContinueLoop) {
                    self.arm_recover_pass_with_targeted_full_schemas(
                        &mut targeted_tools,
                        Some(&response.content),
                        &pre_llm_matched_tools,
                    );
                }
                self.last_llm_ms = llm_ms_acc;
                self.last_tool_ms = tool_ms_acc;
                self.last_total_ms = step_start.elapsed().as_millis() as u64;
                self.broadcast_state().await;
                if matches!(control, TransitionControl::ContinueLoop) {
                    continue;
                }
                return Ok(());
            }

            let parsed = match parse_llm_response_protocol(&response.content) {
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        turn_seq,
                        "LLM response failed protocol JSON parse; omitting assistant stack push and skipping condensation for this hop"
                    );
                    let directive = self.protocol_parse_failure_directive(&e, &response.content);
                    let transition = decide_transition_from_directive(directive);
                    let control = self.apply_transition(transition).await?;
                    if matches!(control, TransitionControl::ContinueLoop) {
                        self.arm_recover_pass_with_targeted_full_schemas(
                            &mut targeted_tools,
                            Some(&response.content),
                            &pre_llm_matched_tools,
                        );
                    }
                    self.last_llm_ms = llm_ms_acc;
                    self.last_tool_ms = tool_ms_acc;
                    self.last_total_ms = step_start.elapsed().as_millis() as u64;
                    self.broadcast_state().await;
                    if matches!(control, TransitionControl::ContinueLoop) {
                        continue;
                    }
                    return Ok(());
                }
                Ok(p) => p,
            };

            let deck_content =
                self.stitch_pending_weather_report_into_content(&response.content);
            self.emit_optional_user_message(&deck_content).await;

            self.chat_stack.push(crate::engine::Message {
                role: "assistant".to_string(),
                content: deck_content,
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
            let model_declared_reflect = parsed.status() == LoopAction::Reflect;
            let directive = self.directive_from_parsed(parsed);
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
                    if model_declared_reflect && self.state != AgentState::Reflect {
                        self.state = AgentState::Reflect;
                        self.broadcast_state().await;
                    }
                    let decision = self
                        .execute_tool_batch(
                            tools,
                            tools_needed,
                            &mut execution_ledger,
                            &mut schema_recovery_attempted,
                            &mut targeted_tools,
                            &mut web_tool_activity,
                            &mut tool_ms_acc,
                            turn_seq,
                            moltbook_overlay_latched,
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
                            continue;
                        }
                        ToolBatchDecision::SuppressOnlyIdlePass { message } => {
                            tracing::info!(
                                event = "orchestrator.tools.duplicate_suppress_idle_pass",
                                "Duplicate-only tool batch; forcing conversational pass without Recover"
                            );
                            self.state = AgentState::Chat;
                            self.chat_stack.push(crate::engine::Message {
                                role: "system".to_string(),
                                content: message,
                            });
                            tools_needed = false;
                            targeted_tools.clear();
                            self.force_full_tool_schemas_in_llm_view = false;
                            continue;
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
                    if self.state == AgentState::Recover {
                        self.arm_recover_pass_with_targeted_full_schemas(
                            &mut targeted_tools,
                            Some(&response.content),
                            &pre_llm_matched_tools,
                        );
                    }
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

#[cfg(test)]
mod trailing_json_recovery_tests {
    use super::trailing_json_recovery_triggered;
    use crate::config::{AppConfig, LlmBackend};

    #[test]
    fn trailing_content_check_skipped_for_grammar() {
        let mut c = AppConfig::default();
        c.llm_backend = LlmBackend::LlamaCpp;
        let raw = r#"{"thought":"t","status":"Idle","message_to_user":"hi","tool_calls":[]}

# Extra"#;
        assert!(!trailing_json_recovery_triggered(&c, raw));
    }

    #[test]
    fn trailing_content_still_triggers_for_ollama_when_tail_present() {
        let c = AppConfig::default();
        let raw = r#"{"thought":"t","status":"Idle","message_to_user":"hi","tool_calls":[]}

# Extra"#;
        assert!(trailing_json_recovery_triggered(&c, raw));
    }
}
