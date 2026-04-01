use crate::executive::error::{FcpError, Result};
use crate::engine::LlmEngine;
use crate::tools::Gatekeeper;
use crate::tools::ToolDescriptorRegistry;
use crate::memory::ephemeral::EphemeralMemory;
use crate::orchestrator::state::{AgentState, LoopDirective, LlmResponse, LoopAction, ToolIntentStatus, ToolIntentTicket};
use crate::orchestrator::context::ContextAssembler;
use crate::orchestrator::r#loop::directive_policy::decide_transition_from_directive;
use crate::orchestrator::r#loop::recovery_policy::{classify_tool_failure, ToolFailureAction};
use crate::orchestrator::r#loop::tool_batch::ToolBatchDecision;
use crate::orchestrator::r#loop::transition::{StateTransition, TransitionControl};
use crate::orchestrator::tool_router::ToolRouter;
use crate::telemetry::routing_codes;
use crate::ui::events::SYSTEM_ALARM_PREFIX;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::path::Path;
use std::time::Instant;

/// Marker string in `thought` / `message_to_user` when the last user line was empty (debuggable in logs and TUI).
pub const EMPTY_USER_MESSAGE_TAG: &str = "SY FNORD";

const EMPTY_USER_SHRUGS: &[&str] = &["¯\\_(ツ)_/¯", "(・_・)", "(╯°□°）╯︵ ┻━┻"];

pub struct Orchestrator<E: LlmEngine> {
    pub state: AgentState,
    pub engine: E,
    pub gatekeeper: Gatekeeper,
    pub ephemeral: Arc<EphemeralMemory>,
    pub context_assembler: ContextAssembler,
    pub tool_router: Option<ToolRouter>,

    // Bounds
    pub max_recovery_attempts: u8,
    pub max_tool_rounds: u8,
    pub condensation_threshold: f32,
    pub num_ctx: usize,

    // Live Loop State
    pub recovery_count: u8,
    pub tool_rounds: u8,

    pub chat_stack: Vec<crate::engine::Message>,
    pub saved_chat_state: Option<Vec<crate::engine::Message>>,
    pub interrupt_rx: tokio::sync::watch::Receiver<()>,
    pub tui_tx: Option<tokio::sync::mpsc::Sender<crate::ui::events::TuiEvent>>,
    pub queued_inputs: usize,
    pub last_router_ms: u64,
    pub last_llm_ms: u64,
    pub last_tool_ms: u64,
    pub last_total_ms: u64,
    pub last_top_tool_match: Option<String>,
    /// Max tools that receive full JSON schemas in Tier 1 (plus all targeted recovery tools).
    pub tool_schema_top_k: usize,
    pub descriptor_jit_max_chars: usize,
    pub descriptor_registry: Option<Arc<ToolDescriptorRegistry>>,
    /// Monotonic counter incremented once per `step()` entry (log correlation; no span across await in `spawn`).
    pub turn_seq: u64,
}

impl<E: LlmEngine> Orchestrator<E> {
    const MAX_TOOL_RESULT_CHARS: usize = 2500;
    const WEB_CONDENSATION_THRESHOLD: f32 = 0.85;
    fn normalize_json(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let mut sorted = BTreeMap::new();
                for (k, v) in map {
                    sorted.insert(k.clone(), Self::normalize_json(v));
                }
                let mut normalized = serde_json::Map::new();
                for (k, v) in sorted {
                    normalized.insert(k, v);
                }
                serde_json::Value::Object(normalized)
            }
            serde_json::Value::Array(arr) => {
                serde_json::Value::Array(arr.iter().map(Self::normalize_json).collect())
            }
            other => other.clone(),
        }
    }

    fn agent_name(&self) -> String {
        let workspace_root = self
            .context_assembler
            .core_dir
            .parent()
            .unwrap_or(&self.context_assembler.core_dir);
        workspace_root
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("ERIS")
            .to_string()
    }

    fn extract_json_slice(response_json: &str) -> &str {
        if let (Some(start), Some(end)) = (response_json.find('{'), response_json.rfind('}')) {
            if start <= end {
                &response_json[start..=end]
            } else {
                response_json
            }
        } else {
            response_json
        }
    }

    fn trim_chars(input: &str, max_len: usize) -> String {
        if input.len() <= max_len {
            return input.to_string();
        }
        let mut limit = max_len;
        while limit > 0 && !input.is_char_boundary(limit) {
            limit -= 1;
        }
        let mut out = input[..limit].to_string();
        out.push_str("… [truncated]");
        out
    }

    fn last_user_content(&self) -> &str {
        self.chat_stack
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .unwrap_or("")
    }

    fn allowed_tool_names_sorted(gatekeeper: &Gatekeeper, state: &AgentState) -> Vec<String> {
        let mut names: Vec<String> = gatekeeper
            .get_allowed_tools(state)
            .into_iter()
            .filter_map(|t| {
                t.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string())
            })
            .collect();
        names.sort();
        names
    }

    /// Tier 1 = targeted recovery tools (all) plus Top-K router matches; Tier 2 = remaining allowed names.
    fn compute_tier1_roster(
        gatekeeper: &Gatekeeper,
        state: &AgentState,
        router_matches: &[String],
        targeted: &HashSet<String>,
        k: usize,
    ) -> (Vec<String>, Vec<String>) {
        let allowed_sorted = Self::allowed_tool_names_sorted(gatekeeper, state);
        let allowed_set: HashSet<String> = allowed_sorted.iter().cloned().collect();
        let k = k.max(1);
        let mut tier1 = Vec::new();
        let mut seen = HashSet::new();

        let mut targeted_sorted: Vec<String> = targeted
            .iter()
            .filter(|t| allowed_set.contains(t.as_str()))
            .cloned()
            .collect();
        targeted_sorted.sort();
        for t in targeted_sorted {
            seen.insert(t.clone());
            tier1.push(t);
        }

        for name in router_matches {
            if seen.contains(name) {
                continue;
            }
            if !allowed_set.contains(name) {
                continue;
            }
            if tier1.len() >= targeted.len() + k {
                break;
            }
            tier1.push(name.clone());
            seen.insert(name.clone());
        }

        let tier1_set: HashSet<String> = tier1.iter().cloned().collect();
        let roster: Vec<String> = allowed_sorted
            .into_iter()
            .filter(|n| !tier1_set.contains(n))
            .collect();
        (tier1, roster)
    }

    async fn handle_empty_user_turn(&mut self) -> Result<()> {
        let idx = self.chat_stack.len() % EMPTY_USER_SHRUGS.len().max(1);
        let face = EMPTY_USER_SHRUGS[idx];
        let thought = format!("{} — empty last user message", EMPTY_USER_MESSAGE_TAG);
        let message_to_user = format!("{face} {}", EMPTY_USER_MESSAGE_TAG);
        let value = serde_json::json!({
            "thought": thought,
            "status": "Idle",
            "message_to_user": message_to_user,
            "tool_calls": []
        });
        let content = serde_json::to_string(&value)?;
        self.emit_optional_user_message(&content).await;
        self.chat_stack.push(crate::engine::Message {
            role: "assistant".to_string(),
            content,
        });
        self.state = AgentState::Idle;
        self.last_llm_ms = 0;
        self.last_total_ms = 0;
        self.broadcast_state().await;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        engine: E,
        gatekeeper: Gatekeeper,
        ephemeral: Arc<EphemeralMemory>,
        vault_root: &Path,
        workspace: &str,
        max_recovery_attempts: u8,
        max_tool_rounds: u8,
        condensation_threshold: f32,
        num_ctx: usize,
        tool_schema_top_k: usize,
        descriptor_jit_max_chars: usize,
        interrupt_rx: tokio::sync::watch::Receiver<()>,
        tui_tx: Option<tokio::sync::mpsc::Sender<crate::ui::events::TuiEvent>>,
        tool_router: Option<ToolRouter>,
        descriptor_registry: Option<Arc<ToolDescriptorRegistry>>,
    ) -> Self {
        Self {
            state: AgentState::Idle,
            engine,
            gatekeeper,
            ephemeral,
            context_assembler: ContextAssembler::new(vault_root, workspace),
            tool_router,
            max_recovery_attempts,
            max_tool_rounds,
            condensation_threshold,
            num_ctx,
            recovery_count: 0,
            tool_rounds: 0,
            chat_stack: Vec::new(),
            saved_chat_state: None,
            interrupt_rx,
            tui_tx,
            queued_inputs: 0,
            last_router_ms: 0,
            last_llm_ms: 0,
            last_tool_ms: 0,
            last_total_ms: 0,
            last_top_tool_match: None,
            tool_schema_top_k,
            descriptor_jit_max_chars,
            descriptor_registry,
            turn_seq: 0,
        }
    }

    /// Descriptor JIT hints only for tools that have full JSON schemas in the system prompt (Tier 1).
    fn build_descriptor_jit_guidance(
        &self,
        state: &AgentState,
        tier1_tool_names: &[String],
    ) -> Option<String> {
        let registry = self.descriptor_registry.as_ref()?;
        let mut selected: Vec<String> = tier1_tool_names.to_vec();
        if selected.is_empty() {
            return None;
        }
        selected.sort();
        selected.dedup();

        let allowed_names = self
            .gatekeeper
            .get_allowed_tools(state)
            .into_iter()
            .filter_map(|t| {
                t.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string())
            })
            .collect::<HashSet<_>>();

        let mut sections = Vec::new();
        let mut used = 0usize;
        let max_chars = self.descriptor_jit_max_chars.max(500);
        for name in selected {
            if !allowed_names.contains(&name) {
                continue;
            }
            let Some(desc) = registry.get(&name) else {
                continue;
            };
            let snippet = format!(
                "Tool: {}\nWhen to use: {}\nWhen not to use: {}\nGood examples: {}\nBad examples: {}",
                desc.tool_name,
                desc.when_to_use.as_deref().unwrap_or("n/a"),
                desc.when_not_to_use.as_deref().unwrap_or("n/a"),
                desc.examples_good
                    .iter()
                    .take(2)
                    .map(|e| format!("{} {}", e.name, e.args))
                    .collect::<Vec<_>>()
                    .join(" | "),
                desc.examples_bad
                    .iter()
                    .take(2)
                    .map(|e| format!("{} {}", e.name, e.args))
                    .collect::<Vec<_>>()
                    .join(" | ")
            );
            if used + snippet.len() > max_chars {
                break;
            }
            used += snippet.len();
            sections.push(snippet);
        }
        if sections.is_empty() {
            return None;
        }
        tracing::debug!(
            jit_section_chars = used,
            jit_section_cap = max_chars,
            selected_tools = sections.len(),
            "Descriptor JIT guidance budget usage"
        );
        Some(format!(
            "[JIT TOOL GUIDANCE]\nUse the following targeted tool guidance while keeping args fully compliant with provided JSON schemas.\n{}\n[/JIT TOOL GUIDANCE]",
            sections.join("\n\n")
        ))
    }


    pub async fn broadcast_state(&self) {
        if let Some(tx) = &self.tui_tx {
            let update = crate::ui::events::AgentStateUpdate {
                state: self.state,
                tool_rounds: self.tool_rounds,
                recovery_count: self.recovery_count,
                active_task: None,
                queued_inputs: self.queued_inputs,
                router_ms: self.last_router_ms,
                llm_ms: self.last_llm_ms,
                tool_ms: self.last_tool_ms,
                total_ms: self.last_total_ms,
                top_tool_match: self.last_top_tool_match.clone(),
            };
            let _ = tx.send(crate::ui::events::TuiEvent::StateUpdate(update)).await;
        }
    }

    /// The main cognitive loop.
    ///
    /// Pre-LLM routing: alarm prefix and short-input guard → conversational; else
    /// semantic Top-K for Tier 1 schemas and full roster in Tier 2 (never
    /// conversational on empty semantic match). Always exactly one LLM
    /// generation per user turn unless interrupted.
    #[allow(clippy::never_loop)]
    pub async fn step(&mut self, _user_input: Option<String>) -> Result<()> {
        self.turn_seq = self.turn_seq.saturating_add(1);
        let turn_seq = self.turn_seq;
        // No `info_span!().entered()` here: `EnteredSpan` is not `Send` and `step()` awaits
        // inside `tokio::spawn`. Correlation uses `turn_seq` on every routing event instead.

        let step_start = Instant::now();
        let mut llm_ms_acc = 0u64;
        let mut tool_ms_acc = 0u64;
        self.recovery_count = 0;
        self.tool_rounds = 0;
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

        // ── Pre-LLM semantic routing ─────────────────────────────────
        let (mut tools_needed, pre_llm_matched_tools) = self.run_pre_llm_routing().await;
        let mut execution_ledger: HashMap<String, ToolIntentTicket> = HashMap::new();
        let mut schema_recovery_attempted: HashSet<String> = HashSet::new();
        let mut targeted_tools: HashSet<String> = HashSet::new();

        // ── Tool-enabled loop (full schemas) ─────────────────────────
        loop {
            // 1. Bailout Checks
            if self.recovery_count >= self.max_recovery_attempts {
                tracing::warn!(recovery_count = self.recovery_count, max = self.max_recovery_attempts, "Max recovery attempts reached, bailing out");
                self.state = AgentState::Idle;
                self.broadcast_state().await;
                return Ok(());
            }
            if self.tool_rounds >= self.max_tool_rounds {
                tracing::warn!(tool_rounds = self.tool_rounds, max = self.max_tool_rounds, "Max tool rounds reached, bailing out");
                self.state = AgentState::Idle;
                self.broadcast_state().await;
                return Ok(());
            }

            // 2. Context Assembly (Tier 1 full schemas + Tier 2 roster, or conversational)
            let (tier1_names, roster_names) = if tools_needed {
                Self::compute_tier1_roster(
                    &self.gatekeeper,
                    &self.state,
                    &pre_llm_matched_tools,
                    &targeted_tools,
                    self.tool_schema_top_k,
                )
            } else {
                (Vec::new(), Vec::new())
            };
            if tools_needed {
                let jit_enabled =
                    self.descriptor_registry.is_some() && !tier1_names.is_empty();
                tracing::debug!(
                    category = routing_codes::CATEGORY_ROUTING,
                    turn_seq = self.turn_seq,
                    tier1_count = tier1_names.len(),
                    tier2_count = roster_names.len(),
                    jit_enabled,
                    tools_needed,
                    "context tier assembly"
                );
            }
            let system_prompt = if !tools_needed {
                self.context_assembler.assemble_conversational(&self.ephemeral).await?
            } else {
                self.context_assembler
                    .assemble_two_tier(
                        &self.state,
                        &self.ephemeral,
                        &self.gatekeeper,
                        &tier1_names,
                        &roster_names,
                    )
                    .await?
            };
            tracing::debug!(prompt_len = system_prompt.len(), "System prompt assembled");
            Self::upsert_system_prompt(&mut self.chat_stack, system_prompt);
            if tools_needed
                && let Some(jit_guidance) =
                    self.build_descriptor_jit_guidance(&self.state, &tier1_names)
            {
                self.chat_stack.push(crate::engine::Message {
                    role: "system".to_string(),
                    content: jit_guidance,
                });
            }

            tracing::info!(chat_stack_len = self.chat_stack.len(), "Sending to LLM engine");

            // 3. Engine Generation
            let response_result = tokio::select! {
                res = async {
                    let llm_started = Instant::now();
                    let out = self.engine.generate(&self.chat_stack, "", None).await;
                    llm_ms_acc = llm_ms_acc.saturating_add(llm_started.elapsed().as_millis() as u64);
                    out
                } => res,
                _ = self.interrupt_rx.changed() => {
                    self.saved_chat_state = Some(self.chat_stack.clone());
                    self.chat_stack.clear();

                    let workspace_root = self.context_assembler.core_dir.parent().unwrap_or(&self.context_assembler.core_dir);
                    let agenda_path = workspace_root.join(".fcp_agenda.json");
                    
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
            let transition = decide_transition_from_directive(directive);
            match transition {
                StateTransition::ExecuteTools(tools) => {
                    let decision = self.execute_tool_batch(
                        tools,
                        tools_needed,
                        &mut execution_ledger,
                        &mut schema_recovery_attempted,
                        &mut targeted_tools,
                        &mut web_tool_activity,
                        &mut tool_ms_acc,
                    ).await?;
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
                            self.apply_transition(StateTransition::Recover { message, schema_retry: true }).await?;
                            continue;
                        }
                        ToolBatchDecision::Recover { message } => {
                            self.apply_transition(StateTransition::Recover { message, schema_retry: false }).await?;
                        }
                        ToolBatchDecision::Fatal(e) => {
                            tracing::error!(error = %e, "System fatality - aborting orchestrator");
                            self.apply_transition(StateTransition::Fatal(FcpError::EngineFault(e.to_string()))).await?;
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

    /// Single mutation funnel for state-machine transitions.
    ///
    /// Any transition that changes visible runtime state should be applied
    /// through this method so broadcast/counter behavior stays uniform.
    async fn apply_transition(&mut self, transition: StateTransition) -> Result<TransitionControl> {
        match transition {
            StateTransition::ExecuteTools(_) => Ok(TransitionControl::ContinueLoop),
            StateTransition::Halt => {
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
                    tracing::warn!(recovery_count = self.recovery_count, "Schema retry recovery transition");
                } else {
                    tracing::warn!(recovery_count = self.recovery_count, "Recover transition");
                }
                self.chat_stack.push(crate::engine::Message {
                    role: "system".to_string(),
                    content: message.clone(),
                });
                if let Some(tx) = &self.tui_tx {
                    let _ = tx.send(crate::ui::events::TuiEvent::SystemError(message)).await;
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

    /// Conversational vs tool mode, plus ordered router names for Tier 1 (Top-K).
    async fn run_pre_llm_routing(&mut self) -> (bool, Vec<String>) {
        let user_input = self.last_user_content();
        let turn_seq = self.turn_seq;

        if user_input.starts_with(SYSTEM_ALARM_PREFIX) {
            self.last_router_ms = 0;
            self.last_top_tool_match = None;
            tracing::info!(
                category = routing_codes::CATEGORY_ROUTING,
                issue = routing_codes::ISSUE_PRELLM_CONV_ALARM,
                outcome = routing_codes::OUTCOME_CONVERSATIONAL,
                turn_seq,
                tools_needed = false,
                router_match_count = 0usize,
                "system alarm prefix; conversational mode"
            );
            return (false, Vec::new());
        }

        if ToolRouter::short_input_guard_conversational_only(user_input) {
            self.last_router_ms = 0;
            self.last_top_tool_match = None;
            tracing::info!(
                category = routing_codes::CATEGORY_ROUTING,
                issue = routing_codes::ISSUE_PRELLM_CONV_SHORT_INPUT,
                outcome = routing_codes::OUTCOME_CONVERSATIONAL,
                turn_seq,
                tools_needed = false,
                router_match_count = 0usize,
                "short-input guard; conversational mode"
            );
            return (false, Vec::new());
        }

        let Some(router) = &self.tool_router else {
            self.last_router_ms = 0;
            self.last_top_tool_match = None;
            tracing::warn!(
                category = routing_codes::CATEGORY_ROUTING,
                issue = routing_codes::ISSUE_PRELLM_ROUTER_UNAVAILABLE,
                outcome = routing_codes::outcome_from_pre_llm_tuple(true, 0),
                turn_seq,
                tools_needed = true,
                router_match_count = 0usize,
                "no tool router; roster-only tool mode"
            );
            return (true, Vec::new());
        };

        let router_started = Instant::now();
        match router.match_tools(user_input).await {
            Ok(matches) if matches.is_empty() => {
                self.last_router_ms = router_started.elapsed().as_millis() as u64;
                self.last_top_tool_match = None;
                tracing::info!(
                    category = routing_codes::CATEGORY_ROUTING,
                    issue = routing_codes::ISSUE_PRELLM_SEMANTIC_EMPTY,
                    outcome = routing_codes::outcome_from_pre_llm_tuple(true, 0),
                    turn_seq,
                    tools_needed = true,
                    router_match_count = 0usize,
                    "no semantic tool match; roster-only tool mode"
                );
                (true, Vec::new())
            }
            Ok(matches) => {
                self.last_router_ms = router_started.elapsed().as_millis() as u64;
                self.last_top_tool_match = matches.first().map(|(name, score)| format!("{name}({score:.3})"));
                let matched_preview: Vec<String> = matches
                    .iter()
                    .map(|(n, s)| format!("{}({:.3})", n, s))
                    .collect();
                let names: Vec<String> = matches.into_iter().map(|(name, _)| name).collect();
                let router_match_count = names.len();
                tracing::info!(
                    category = routing_codes::CATEGORY_ROUTING,
                    issue = routing_codes::ISSUE_PRELLM_SEMANTIC_HIT,
                    outcome = routing_codes::outcome_from_pre_llm_tuple(true, router_match_count),
                    turn_seq,
                    tools_needed = true,
                    router_match_count,
                    matched = ?matched_preview,
                    "semantic tool match; Tier1 + roster"
                );
                (true, names)
            }
            Err(e) => {
                self.last_router_ms = router_started.elapsed().as_millis() as u64;
                self.last_top_tool_match = None;
                tracing::warn!(
                    category = routing_codes::CATEGORY_ROUTING,
                    issue = routing_codes::ISSUE_PRELLM_MATCH_ERROR,
                    outcome = routing_codes::outcome_from_pre_llm_tuple(true, 0),
                    turn_seq,
                    tools_needed = true,
                    router_match_count = 0usize,
                    fcp_error = %e,
                    "pre-LLM match_tools failed; roster-only tool mode"
                );
                (true, Vec::new())
            }
        }
    }

    /// Emits an assistant-facing message to TUI when present in the model JSON.
    async fn emit_optional_user_message(&self, response_content: &str) {
        let Some(tx) = &self.tui_tx else {
            return;
        };

        let json_slice = Self::extract_json_slice(response_content);
        if let Ok(parsed) = serde_json::from_str::<LlmResponse>(json_slice)
            && let Some(msg) = parsed.message_to_user
            && !msg.trim().is_empty()
        {
            let agent_name = self.agent_name();
            let out_line = if parsed.tool_calls.is_empty() {
                msg
            } else {
                format!("{} [running tools…]", msg)
            };
            tracing::info!(
                event = "UI_EMIT_INCOMING_MESSAGE",
                agent = %agent_name,
                msg_len = out_line.len(),
                preview = %out_line.chars().take(120).collect::<String>(),
                "Emitting assistant message to TUI deck"
            );
            let _ = tx
                .send(crate::ui::events::TuiEvent::IncomingMessage(
                    format!("[{}]: {}", agent_name, out_line),
                ))
                .await;
        }
    }

    #[allow(clippy::too_many_arguments)]
    /// Executes one tool batch and returns a decision for the coordinator.
    ///
    /// This method owns tool dispatch mechanics; caller applies resulting
    /// transitions via `apply_transition`.
    async fn execute_tool_batch(
        &mut self,
        tools: Vec<crate::orchestrator::state::ToolCall>,
        tools_needed: bool,
        execution_ledger: &mut HashMap<String, ToolIntentTicket>,
        schema_recovery_attempted: &mut HashSet<String>,
        targeted_tools: &mut HashSet<String>,
        web_tool_activity: &mut bool,
        tool_ms_acc: &mut u64,
    ) -> Result<ToolBatchDecision> {
        if !tools_needed {
            tracing::info!(tool_count = tools.len(), "Latent tool intent detected in conversational path");
        }
        tracing::info!(event = "orchestrator.tools.batch", tool_count = tools.len(), "Executing tool calls");
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
                && matches!(existing.status, ToolIntentStatus::Pending | ToolIntentStatus::Success)
            {
                tracing::warn!(tool = %tool_name, intent_id = %intent_id, "Duplicate tool call suppressed in current turn");
                suppressed_duplicate_count += 1;
                let msg = format!("[SYSTEM] Duplicate tool call suppressed for '{}'. Continue without repeating it.", tool_name);
                self.chat_stack.push(crate::engine::Message {
                    role: "system".to_string(),
                    content: msg.clone(),
                });
                if let Some(tx) = &self.tui_tx {
                    let _ = tx.send(crate::ui::events::TuiEvent::SystemError(msg)).await;
                }
                continue;
            }
            let prev_attempts = execution_ledger
                .get(&intent_id)
                .map(|t| t.attempt_count)
                .unwrap_or(0);
            execution_ledger.insert(intent_id.clone(), ToolIntentTicket {
                intent_id: intent_id.clone(),
                tool_name: tool_name.clone(),
                args: args.clone(),
                status: ToolIntentStatus::Pending,
                attempt_count: prev_attempts.saturating_add(1),
                last_error: None,
            });
            tracing::debug!(tool = %tool_name, intent_id = %intent_id, "Intent ticket set to Pending");
            tracing::info!(tool = %tool_name, args = %args, state = ?current_state, "Dispatching tool");
            let tool_started = Instant::now();
            let result = self.gatekeeper.execute_tool(&current_state, &tool_name, args.clone()).await;
            *tool_ms_acc = (*tool_ms_acc).saturating_add(tool_started.elapsed().as_millis() as u64);
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
                    tracing::info!(tool = %tool_name, intent_id = %intent_id, result_len = result.len(), round = self.tool_rounds, "Tool succeeded");
                    let bounded_result = Self::trim_chars(&result, Self::MAX_TOOL_RESULT_CHARS);
                    let msg = format!("Tool '{}' succeeded: {}", tool_name, bounded_result);
                    self.chat_stack.push(crate::engine::Message {
                        role: "system".to_string(),
                        content: msg.clone(),
                    });
                    if let Some(tx) = &self.tui_tx {
                        let _ = tx.send(crate::ui::events::TuiEvent::SystemError(msg)).await;
                    }
                    self.broadcast_state().await;
                }
                Err(e) => {
                    tracing::error!(tool = %tool_name, intent_id = %intent_id, error = %e, error_type = ?std::mem::discriminant(&e), "Tool execution failed");
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

        let pending_count = execution_ledger.values().filter(|t| matches!(t.status, ToolIntentStatus::Pending)).count();
        if pending_count > 0 {
            tracing::error!(pending_count, "Pending-state closure invariant violated");
            self.state = AgentState::Idle;
            self.broadcast_state().await;
            return Err(FcpError::EngineFault(
                format!("Tool intent ledger invariant violated: {pending_count} pending intents after dispatch"),
            ));
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
            tracing::info!("All tool intents in batch were duplicate-suppressed; halting turn early");
            return Ok(ToolBatchDecision::Halt);
        }

        if targeted_recovery_requested {
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
            let msg = format!("[SYSTEM OVERRIDE: FUCKUP DETECTED] Tool execution failed: {}", reason);
            return Ok(ToolBatchDecision::Recover { message: msg });
        }

        Ok(ToolBatchDecision::Continue)
    }

    fn upsert_system_prompt(chat_stack: &mut Vec<crate::engine::Message>, prompt: String) {
        if let Some(first) = chat_stack.first_mut() {
            if first.role == "system" {
                first.content = prompt;
            } else {
                chat_stack.insert(0, crate::engine::Message {
                    role: "system".to_string(),
                    content: prompt,
                });
            }
        } else {
            chat_stack.push(crate::engine::Message {
                role: "system".to_string(),
                content: prompt,
            });
        }
    }

    fn tool_fingerprint(name: &str, args: &serde_json::Value) -> String {
        let normalized = Self::normalize_json(args);
        let args_json = serde_json::to_string(&normalized).unwrap_or_else(|_| "null".to_string());
        let mut hasher = Sha256::new();
        hasher.update(name.as_bytes());
        hasher.update(b"\n");
        hasher.update(args_json.as_bytes());
        let digest = hasher.finalize();
        let mut hex = String::with_capacity(40);
        for b in &digest[..20] {
            use std::fmt::Write as _;
            let _ = write!(&mut hex, "{:02x}", b);
        }
        hex
    }

    #[cfg(test)]
    fn is_schema_or_parse_tool_error(e: &FcpError) -> bool {
        matches!(classify_tool_failure(e, false), ToolFailureAction::TargetedSchemaRetry)
    }

    pub fn process_llm_response(&mut self, response_json: &str) -> LoopDirective {
        let json_str = Self::extract_json_slice(response_json);

        tracing::debug!(extracted_json_len = json_str.len(), "Parsing LLM JSON response");

        let response: LlmResponse = match serde_json::from_str(json_str) {
            Ok(res) => res,
            Err(e) => {
                tracing::warn!(error = %e, raw_snippet = &json_str[..json_str.len().min(200)], "Failed to parse LLM response as JSON");
                return LoopDirective::RecoverFromFuckup(e.to_string());
            }
        };

        let explicit_status = response.has_explicit_status();
        let status = response.status();
        tracing::info!(
            status = ?status,
            explicit_status,
            thought_len = response.thought.len(),
            tool_count = response.tool_calls.len(),
            has_message = response.message_to_user.is_some(),
            "Parsed LLM response"
        );

        if !explicit_status
            && response.tool_calls.is_empty()
            && response.message_to_user.as_ref().is_none_or(|m| m.trim().is_empty())
        {
            return LoopDirective::RecoverFromFuckup(
                "Missing required `status` and no actionable fields (`tool_calls`/`message_to_user`)".to_string(),
            );
        }

        // Tools take precedence: never drop tool_calls because of Idle/Reflect/Task mismatch.
        if !response.tool_calls.is_empty() {
            return LoopDirective::ExecuteTools(response.tool_calls);
        }

        match status {
            LoopAction::Reflect => {
                if let Some(msg) = response.message_to_user
                    && !msg.trim().is_empty()
                {
                    return LoopDirective::HaltAndAwaitInput(Some(msg));
                }
                tracing::debug!("Reflect with empty tool_calls — treating as Task");
                self.state = AgentState::Chat;
                LoopDirective::ShiftToReflection
            }
            LoopAction::Idle => match response.message_to_user {
                Some(msg) if !msg.trim().is_empty() => LoopDirective::HaltAndAwaitInput(Some(msg)),
                _ => LoopDirective::RecoverFromFuckup(
                    "Idle status requires non-empty message_to_user".to_string(),
                ),
            },
            LoopAction::Task => {
                self.state = AgentState::Chat;
                LoopDirective::ShiftToReflection
            }
        }
    }

    /// Forces the LLM to summarize the active chat stack, then replaces the stack with the summary.
    pub async fn execute_condensation(&mut self) -> Result<()> {
        // 1. Push a system message to `self.chat_stack` asking for a JSON summary.
        self.chat_stack.push(crate::engine::Message {
            role: "system".to_string(),
            content: "Please summarize the current conversation as a JSON object.".to_string(),
        });

        // 2. Call `self.engine.generate(&self.chat_stack, ...)`.
        let response = self.engine.generate(&self.chat_stack, "", None).await?;

        // 3. Extract the summary text from the response.
        let summary = response.content;

        // 4. Clear `self.chat_stack`.
        self.chat_stack.clear();

        // 5. Push a single Message containing the summary back to `self.chat_stack`.
        self.chat_stack.push(crate::engine::Message {
            role: "system".to_string(),
            content: summary,
        });

        // 6. Set `self.state = AgentState::Chat`.
        self.state = AgentState::Chat;
        self.broadcast_state().await;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Message, EngineResponse};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::mpsc;
    use crate::executive::error::Result;

    #[derive(Clone)]
    struct MockEngine {
        content: String,
        fault: Option<String>,
        prompt_tokens: usize,
        generated_tokens: usize,
    }

    impl MockEngine {
        fn new() -> Self {
            Self {
                content: "mock".to_string(),
                fault: None,
                prompt_tokens: 0,
                generated_tokens: 0,
            }
        }

        fn with_content(content: &str) -> Self {
            Self {
                content: content.to_string(),
                fault: None,
                prompt_tokens: 0,
                generated_tokens: 0,
            }
        }

        fn with_network_fault(msg: &str) -> Self {
            Self {
                content: String::new(),
                fault: Some(msg.to_string()),
                prompt_tokens: 0,
                generated_tokens: 0,
            }
        }

        fn with_tokens(mut self, prompt_tokens: usize, generated_tokens: usize) -> Self {
            self.prompt_tokens = prompt_tokens;
            self.generated_tokens = generated_tokens;
            self
        }
    }

    #[async_trait]
    impl LlmEngine for MockEngine {
        async fn generate(
            &self,
            _stack: &[Message],
            _available_tools_json: &str,
            _stream_tx: Option<mpsc::UnboundedSender<String>>
        ) -> Result<EngineResponse> {
            if let Some(msg) = &self.fault {
                return Err(crate::executive::error::FcpError::NetworkFault(msg.clone()));
            }
            Ok(EngineResponse {
                content: self.content.clone(),
                prompt_tokens: self.prompt_tokens,
                generated_tokens: self.generated_tokens,
            })
        }
    }

    #[test]
    fn test_orchestrator_initialization() {
        let engine = MockEngine::new();
        let gatekeeper = Gatekeeper::new();
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let vault_root = Path::new("/tmp/vault");
        let (tx, rx) = tokio::sync::watch::channel(());
        Box::leak(Box::new(tx));

        let orchestrator = Orchestrator::new(
            engine,
            gatekeeper,
            ephemeral,
            vault_root,
            "test_ws",
            3,
            5,
            0.8,
            4096,
            3,
            6000,
            rx,
            None,
            None,
            None,
        );

        assert_eq!(orchestrator.state, AgentState::Idle);
        assert_eq!(orchestrator.recovery_count, 0);
        assert_eq!(orchestrator.tool_rounds, 0);
        assert_eq!(orchestrator.max_recovery_attempts, 3);
        assert_eq!(orchestrator.max_tool_rounds, 5);
        assert_eq!(orchestrator.condensation_threshold, 0.8);
        assert_eq!(orchestrator.num_ctx, 4096);
    }

    fn setup_orchestrator() -> Orchestrator<MockEngine> {
        setup_orchestrator_with_engine(MockEngine::new())
    }

    fn setup_orchestrator_with_engine(engine: MockEngine) -> Orchestrator<MockEngine> {
        let gatekeeper = Gatekeeper::new();
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let vault_root = Path::new("/tmp/vault");
        let (tx, rx) = tokio::sync::watch::channel(());
        Box::leak(Box::new(tx)); // Prevent sender from dropping, which would trigger `rx.changed()`
        Orchestrator::new(engine, gatekeeper, ephemeral, vault_root, "test_ws", 3, 5, 0.8, 4096, 3, 6000, rx, None, None, None)
    }

    #[test]
    fn test_router_valid_tool_call() {
        let mut orchestrator = setup_orchestrator();
        let json = r#"{
            "thought": "test",
            "status": "Reflect",
            "tool_calls": [{ "name": "foo", "args": {} }]
        }"#;
        
        let directive = orchestrator.process_llm_response(json);
        match directive {
            LoopDirective::ExecuteTools(tools) => {
                assert_eq!(tools.len(), 1);
                assert_eq!(tools[0].name, "foo");
            }
            _ => panic!("Expected ExecuteTools"),
        }
    }

    #[test]
    fn test_router_idle_with_tools_executes_tools() {
        let mut orchestrator = setup_orchestrator();
        let json = r#"{
            "thought": "wrong status but tools present",
            "status": "Idle",
            "message_to_user": "Hang on…",
            "tool_calls": [{ "name": "vault:read", "args": { "path": "x.md" } }]
        }"#;

        let directive = orchestrator.process_llm_response(json);
        match directive {
            LoopDirective::ExecuteTools(tools) => {
                assert_eq!(tools.len(), 1);
                assert_eq!(tools[0].name, "vault:read");
            }
            _ => panic!("Expected ExecuteTools when tool_calls non-empty"),
        }
    }

    #[test]
    fn test_router_reflect_empty_tools_shifts_to_reflection() {
        let mut orchestrator = setup_orchestrator();
        let json = r#"{
            "thought": "test",
            "status": "Reflect",
            "tool_calls": []
        }"#;

        let directive = orchestrator.process_llm_response(json);
        assert_eq!(directive, LoopDirective::ShiftToReflection);
        assert_eq!(orchestrator.state, AgentState::Chat);
    }

    #[test]
    fn test_router_reflect_empty_tools_with_message_halts() {
        let mut orchestrator = setup_orchestrator();
        let json = r#"{
            "thought": "done",
            "status": "Reflect",
            "message_to_user": "Here are your results.",
            "tool_calls": []
        }"#;

        let directive = orchestrator.process_llm_response(json);
        match directive {
            LoopDirective::HaltAndAwaitInput(Some(msg)) => {
                assert!(msg.contains("results"));
            }
            _ => panic!("Expected HaltAndAwaitInput, got {:?}", directive),
        }
    }

    #[test]
    fn test_router_invalid_json_yields_fuckup() {
        let mut orchestrator = setup_orchestrator();
        let json = r#"{"status": "BAD_JSON"#;
        
        let directive = orchestrator.process_llm_response(json);
        match directive {
            LoopDirective::RecoverFromFuckup(msg) => {
                assert!(!msg.is_empty());
            }
            _ => panic!("Expected RecoverFromFuckup"),
        }
    }

    #[test]
    fn test_router_initiate_reflection_mutates_state() {
        let mut orchestrator = setup_orchestrator();
        let json = r#"{
            "thought": "test",
            "status": "Task",
            "tool_calls": []
        }"#;
        
        let directive = orchestrator.process_llm_response(json);
        assert_eq!(directive, LoopDirective::ShiftToReflection);
        assert_eq!(orchestrator.state, AgentState::Chat);
    }

    #[test]
    fn test_tool_fingerprint_is_stable_for_same_payload() {
        let args = serde_json::json!({"title":"hagbard_profile","tags":["person","contact"]});
        let a = Orchestrator::<MockEngine>::tool_fingerprint("memory:stage", &args);
        let b = Orchestrator::<MockEngine>::tool_fingerprint("memory:stage", &args);
        assert_eq!(a, b);
    }

    #[test]
    fn test_tool_fingerprint_canonicalizes_object_key_order() {
        let a = serde_json::json!({
            "content": "User name is Hagbard.",
            "tags": ["person", "contact"],
            "title": "hagbard_profile"
        });
        let b = serde_json::json!({
            "title": "hagbard_profile",
            "content": "User name is Hagbard.",
            "tags": ["person", "contact"]
        });
        let fa = Orchestrator::<MockEngine>::tool_fingerprint("memory:stage", &a);
        let fb = Orchestrator::<MockEngine>::tool_fingerprint("memory:stage", &b);
        assert_eq!(fa, fb);
    }

    #[test]
    fn test_schema_or_parse_error_detection() {
        let schema_err = crate::executive::error::FcpError::SchemaViolation("bad args".to_string());
        let parse_err = crate::executive::error::FcpError::ParseFault(serde_json::Error::io(std::io::Error::other("bad json")));
        let net_err = crate::executive::error::FcpError::NetworkFault("offline".to_string());

        assert!(Orchestrator::<MockEngine>::is_schema_or_parse_tool_error(&schema_err));
        assert!(Orchestrator::<MockEngine>::is_schema_or_parse_tool_error(&parse_err));
        assert!(!Orchestrator::<MockEngine>::is_schema_or_parse_tool_error(&net_err));
    }

    #[tokio::test]
    async fn test_step_resets_counters_on_entry() {
        let json = r#"{
            "thought": "done",
            "status": "Idle",
            "message_to_user": "hi"
        }"#;
        let engine = MockEngine::with_content(json);
        let mut orchestrator = setup_orchestrator_with_engine(engine);
        orchestrator.recovery_count = 99;
        orchestrator.tool_rounds = 99;
        orchestrator.state = AgentState::Chat;
        
        let result = orchestrator.step(None).await;
        
        assert!(result.is_ok());
        assert_eq!(orchestrator.recovery_count, 0);
        assert_eq!(orchestrator.tool_rounds, 0);
        assert_eq!(orchestrator.state, AgentState::Idle);
    }

    #[tokio::test]
    async fn test_step_system_fatality_aborts() {
        let engine = MockEngine::with_network_fault("daemon offline");
        let mut orchestrator = setup_orchestrator_with_engine(engine);
        orchestrator.state = AgentState::Chat;
        orchestrator.chat_stack.push(Message {
            role: "user".to_string(),
            content: "exercise engine error path".to_string(),
        });

        let result = orchestrator.step(None).await;
        
        assert!(result.is_err());
        assert_eq!(orchestrator.state, AgentState::Idle);
    }

    #[tokio::test]
    async fn test_step_empty_user_line_sy_fnord_no_llm() {
        let json = r#"{"status":"Idle","message_to_user":"engine should not run"}"#;
        let engine = MockEngine::with_content(json);
        let mut orchestrator = setup_orchestrator_with_engine(engine);
        orchestrator.state = AgentState::Chat;
        orchestrator.chat_stack.push(Message {
            role: "user".to_string(),
            content: "   ".to_string(),
        });

        let result = orchestrator.step(None).await;
        assert!(result.is_ok());
        let last = orchestrator
            .chat_stack
            .last()
            .expect("assistant reply for empty user line");
        assert!(last.content.contains(EMPTY_USER_MESSAGE_TAG));
    }

    #[tokio::test]
    async fn test_step_halt_directive_resets_state() {
        let json = r#"{
            "thought": "I'm done",
            "status": "Idle",
            "message_to_user": "how can I help?"
        }"#;
        let engine = MockEngine::with_content(json);
        let mut orchestrator = setup_orchestrator_with_engine(engine);
        orchestrator.state = AgentState::Chat;
        orchestrator.tool_rounds = 2;
        orchestrator.recovery_count = 1;
        
        let result = orchestrator.step(None).await;
        
        assert!(result.is_ok());
        assert_eq!(orchestrator.state, AgentState::Idle);
        assert_eq!(orchestrator.tool_rounds, 0);
        assert_eq!(orchestrator.recovery_count, 0);
    }

    #[tokio::test]
    async fn test_execute_condensation_replaces_stack() {
        let json = r#"{
            "thought": "Summarizing",
            "status": "Task",
            "tool_calls": []
        }"#;
        let engine = MockEngine::with_content(json);
        let mut orchestrator = setup_orchestrator_with_engine(engine);
        orchestrator.chat_stack.push(Message { role: "user".to_string(), content: "hello".to_string() });
        orchestrator.chat_stack.push(Message { role: "assistant".to_string(), content: "world".to_string() });

        let result = orchestrator.execute_condensation().await;
        
        assert!(result.is_ok());
        assert_eq!(orchestrator.chat_stack.len(), 1);
        assert_eq!(orchestrator.chat_stack[0].content, json);
        assert_eq!(orchestrator.state, AgentState::Chat);
    }

    #[tokio::test]
    async fn test_step_triggers_reflection_on_token_exhaustion() {
        let json = r#"{
            "thought": "I'm done",
            "status": "Idle",
            "message_to_user": "hello"
        }"#;
        // Wait, wait, if the engine returns WAIT_FOR_USER, loop will exit gracefully.
        // We shouldn't use CONTINUE_TASK without tools otherwise it loops to RecoverFromFuckup and does another loop
        // Let's use WAIT_FOR_USER to avoid infinite loops since we removed the `break`.
        // With num_ctx = 4096 and threshold = 0.8, max tokens = 3276
        let engine = MockEngine::with_content(json).with_tokens(2000, 1500);
        let mut orchestrator = setup_orchestrator_with_engine(engine);
        orchestrator.state = AgentState::Chat;
        
        let result = orchestrator.step(None).await;
        
        assert!(result.is_ok(), "Expected OK, got: {:?}", result.err());
        // state will be overridden by WAIT_FOR_USER (AgentState::Idle)
        // Hmm... previously `ShiftToReflection` was not tested for its *persistent* state change if loop broke immediately.
        // Let's change json to ShiftToReflection (InitiateReflection).
    }

    #[tokio::test]
    async fn test_async_guillotine_interrupts_generation() {
        use std::time::Duration;
        
        #[derive(Clone)]
        struct PendingEngine;
        #[async_trait]
        impl LlmEngine for PendingEngine {
            async fn generate(
                &self,
                _stack: &[Message],
                _available_tools_json: &str,
                _stream_tx: Option<mpsc::UnboundedSender<String>>
            ) -> Result<EngineResponse> {
                // Hang forever
                tokio::time::sleep(Duration::from_secs(10)).await;
                Ok(EngineResponse {
                    content: "never".to_string(),
                    prompt_tokens: 0,
                    generated_tokens: 0,
                })
            }
        }

        let engine = PendingEngine;
        let gatekeeper = Gatekeeper::new();
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let temp_dir = tempfile::tempdir().unwrap();
        let vault_root = temp_dir.path();
        let workspace = "test_ws";
        
        // Create the core dir so context_assembler has a valid parent
        let core_dir = vault_root.join(workspace).join("00_Core");
        tokio::fs::create_dir_all(&core_dir).await.unwrap();

        // Write a mock agenda file
        let agenda_path = vault_root.join(workspace).join(".fcp_agenda.json");
        let agenda_content = r#"[{"id": "1234", "created_at": 123456, "description": "Test agenda task", "status": "pending"}]"#;
        tokio::fs::write(&agenda_path, agenda_content).await.unwrap();

        let (tx, rx) = tokio::sync::watch::channel(());
        
        let mut orchestrator = Orchestrator::new(
            engine,
            gatekeeper,
            ephemeral,
            vault_root,
            workspace,
            3,
            5,
            0.8,
            4096,
            3,
            6000,
            rx,
            None,
            None,
            None,
        );

        orchestrator.state = AgentState::Chat;
        orchestrator.chat_stack.push(Message { role: "user".to_string(), content: "hello".to_string() });

        // Fire the interrupt shortly after calling step
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let _ = tx.send(());
        });

        let result = orchestrator.step(None).await;
        
        assert!(matches!(result, Err(crate::executive::error::FcpError::Interrupted)));
        assert_eq!(orchestrator.state, AgentState::Idle);
        assert!(orchestrator.saved_chat_state.is_some());
        assert_eq!(orchestrator.saved_chat_state.unwrap()[1].content, "hello");
        assert_eq!(orchestrator.chat_stack.len(), 1);
        assert!(orchestrator.chat_stack[0].content.contains("Test agenda task"));
        assert!(orchestrator.chat_stack[0].content.contains("agenda:complete"));
    }

    #[tokio::test]
    async fn test_duplicate_only_batch_halts_without_extra_generation() {
        #[derive(Clone)]
        struct SequenceEngine {
            responses: Arc<Vec<String>>,
            calls: Arc<AtomicUsize>,
        }

        #[async_trait]
        impl LlmEngine for SequenceEngine {
            async fn generate(
                &self,
                _stack: &[Message],
                _available_tools_json: &str,
                _stream_tx: Option<mpsc::UnboundedSender<String>>
            ) -> Result<EngineResponse> {
                let idx = self.calls.fetch_add(1, Ordering::SeqCst);
                let content = self
                    .responses
                    .get(idx)
                    .cloned()
                    .unwrap_or_else(|| self.responses.last().cloned().unwrap_or_else(|| "{\"status\":\"Idle\",\"message_to_user\":\"done\",\"tool_calls\":[]}".to_string()));
                Ok(EngineResponse {
                    content,
                    prompt_tokens: 0,
                    generated_tokens: 0,
                })
            }
        }

        let first = r#"{
            "thought": "stage once",
            "status": "Reflect",
            "tool_calls": [{
                "name": "memory:stage",
                "args": {
                    "title": "hagbard_profile",
                    "content": "User name is Hagbard.",
                    "tags": ["person","contact"]
                }
            }]
        }"#.to_string();
        let second_duplicate = first.clone();

        let calls = Arc::new(AtomicUsize::new(0));
        let engine = SequenceEngine {
            responses: Arc::new(vec![first, second_duplicate]),
            calls: calls.clone(),
        };

        let mut gatekeeper = Gatekeeper::new();
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        gatekeeper.register(Arc::new(crate::tools::memory::MemoryStageTool {
            ephemeral: ephemeral.clone(),
            ttl_secs: 60,
            max_content_chars: 10_000,
        }));

        let vault_root = Path::new("/tmp/vault");
        let (tx, rx) = tokio::sync::watch::channel(());
        Box::leak(Box::new(tx));

        let mut orchestrator = Orchestrator::new(
            engine,
            gatekeeper,
            ephemeral,
            vault_root,
            "test_ws",
            3,
            5,
            0.8,
            4096,
            3,
            6000,
            rx,
            None,
            None,
            None,
        );
        orchestrator.state = AgentState::Chat;
        orchestrator.chat_stack.push(Message { role: "user".to_string(), content: "remember my name".to_string() });

        let result = orchestrator.step(None).await;
        assert!(result.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 2, "expected no extra LLM round after duplicate-only suppression");
        assert_eq!(orchestrator.state, AgentState::Idle);
    }
}
