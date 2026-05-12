use crate::config::AppConfig;
use crate::engine::LlmEngine;
use crate::memory::ephemeral::EphemeralMemory;
use crate::orchestrator::context::ContextAssembler;
use crate::orchestrator::context::ContextViewSettings;
use crate::orchestrator::state::AgentState;
use crate::orchestrator::tool_router::ToolRouter;
use crate::presentation::{AgentStateUpdate, SessionEvent};
use crate::tools::Gatekeeper;
use crate::tools::ToolDescriptorRegistry;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::moltbook_browse_ledger::MoltbookBrowseLedger;

/// Marker string in `thought` / `message_to_user` when the last user line was empty (debuggable in logs and TUI).
pub const EMPTY_USER_MESSAGE_TAG: &str = "SY FNORD";

/// Injected when [`Orchestrator::max_tool_rounds`] is reached: one final conversational LLM pass (no tool schemas, no descriptor JIT).
pub(crate) const TOOL_ROUND_CAP_SYSTEM_GUIDANCE: &str = r#"[SYSTEM — TOOL BUDGET]
This user turn has reached the configured maximum number of successful tool executions. You cannot call tools again until the user sends a new message.

Respond with a single JSON object in the usual protocol. Use empty `tool_calls` []. Prefer `status` "Idle" with a non-empty `message_to_user` that summarizes what you learned from tool results already in the thread and tells the user they can say **continue** (or similar) if more tool work is needed.

Do not request tools; they will not run."#;

/// Appended when the model still emitted `tool_calls` after the cap recovery pass.
pub(crate) const TOOL_ROUND_CAP_USER_FOOTNOTE: &str = "(Per-turn tool limit reached; further tool calls were not executed. Send another message to continue with tools.)";

/// Deck line when [`Orchestrator::max_recovery_attempts`] is exhausted mid-turn (mirrors tool-cap footnote style).
pub(crate) const RECOVERY_BUDGET_EXHAUSTED_DECK_LINE: &str = "(Recovery budget exhausted this turn; assistant is idle. Send a new message or simplify the request.)";

/// RAII: sets [`Orchestrator::promotion_suppressed_during_step`] for the whole `step()` await tree.
pub(crate) struct PromotionSuppressedDuringStep {
    flag: Arc<AtomicBool>,
}

impl PromotionSuppressedDuringStep {
    pub(crate) fn arm(flag: Arc<AtomicBool>) -> Self {
        flag.store(true, Ordering::SeqCst);
        Self { flag }
    }
}

impl Drop for PromotionSuppressedDuringStep {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::SeqCst);
    }
}

pub struct Orchestrator<E: LlmEngine> {
    pub state: AgentState,
    pub engine: E,
    pub gatekeeper: Gatekeeper,
    pub ephemeral: Arc<EphemeralMemory>,
    pub config: Arc<AppConfig>,
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
    /// Interactive chat must use `Some`; `None` drops outbound deck/state/telemetry (headless tests, batch).
    pub presentation_tx: Option<tokio::sync::mpsc::Sender<SessionEvent>>,
    pub queued_inputs: usize,
    pub last_router_ms: u64,
    pub last_llm_ms: u64,
    pub last_tool_ms: u64,
    pub last_total_ms: u64,
    pub last_top_tool_match: Option<String>,
    /// Whether the most recent LLM generation was executed in tool-enabled mode (tool schemas in prompt).
    /// Used to enforce stricter response invariants only when tools are available.
    pub last_turn_tools_enabled: bool,
    pub descriptor_jit_top_k: usize,
    pub descriptor_jit_max_chars: usize,
    /// Phrase map + tool defs without parameters when true (`slim_tool_prompt` in config).
    pub slim_tool_prompt: bool,
    /// Cap semantic router hits included in slim map (`0` = no cap).
    pub tool_map_offer_cap: usize,
    pub descriptor_registry: Option<Arc<ToolDescriptorRegistry>>,
    /// LLM-only stack transform; stored [`Self::chat_stack`] is unchanged.
    pub context_view: ContextViewSettings,
    /// When true, next [`build_llm_view`] uses full `parameters` in the tool-def block (overrides slim view).
    /// Set after a Gatekeeper schema fault when [`ToolBatchDecision::RetryWithTargetedSchema`] runs; cleared at [`Self::step`] entry and after any successful tool execution in a batch that returns [`ToolBatchDecision::Continue`].
    pub force_full_tool_schemas_in_llm_view: bool,
    /// Monotonic counter incremented once per `step()` entry (log correlation; no span across await in `spawn`).
    pub turn_seq: u64,
    /// Shown in TUI Status while tools are pending; cleared when a final deck message is emitted or at `step` entry.
    pub activity_line: Option<String>,
    /// Last `message_to_user` body sent to the TUI deck this `step()`; avoids duplicate bubbles when Task → Reflect replays the same line.
    pub(crate) last_deck_message_body: Option<String>,
    /// After [`Self::max_tool_rounds`] successful tool runs in this `step()`, the next loop iteration runs one final conversational generation (no tools / no JIT), then idles.
    pub(crate) tool_round_cap_final_pass_pending: bool,
    /// Shared with [`crate::memory::ephemeral::spawn_snapshot_daemon`]: while `true`, promotion/decay ticks are skipped.
    pub(crate) promotion_suppressed_during_step: Arc<AtomicBool>,
    /// When set, successful tool calls update browse-cycle counters and may inject invariant nudges.
    pub(super) moltbook_browse_ledger: Option<MoltbookBrowseLedger>,
    /// Per-`step()` consecutive failure counts for `(tool_name, intent_id)` on repeatable tools (Moltbook latch).
    pub(super) tool_repeat_failure_streak: HashMap<String, u8>,
    /// Tool names that failed in the current `step()`; used to prioritize recovery JIT skill guidance.
    pub(super) step_failed_tools: HashSet<String>,
}

impl<E: LlmEngine> Orchestrator<E> {
    pub async fn broadcast_state(&self) {
        if let Some(tx) = &self.presentation_tx {
            let update = AgentStateUpdate {
                state: self.state,
                tool_rounds: self.tool_rounds,
                max_tool_rounds: self.max_tool_rounds,
                recovery_count: self.recovery_count,
                max_recovery_attempts: self.max_recovery_attempts,
                active_task: None,
                activity_line: self.activity_line.clone(),
                queued_inputs: self.queued_inputs,
                router_ms: self.last_router_ms,
                llm_ms: self.last_llm_ms,
                tool_ms: self.last_tool_ms,
                total_ms: self.last_total_ms,
                top_tool_match: self.last_top_tool_match.clone(),
            };
            let _ = tx.send(SessionEvent::StateUpdate(update)).await;
        }
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
        descriptor_jit_top_k: usize,
        descriptor_jit_max_chars: usize,
        slim_tool_prompt: bool,
        tool_map_offer_cap: usize,
        interrupt_rx: tokio::sync::watch::Receiver<()>,
        presentation_tx: Option<tokio::sync::mpsc::Sender<SessionEvent>>,
        tool_router: Option<ToolRouter>,
        descriptor_registry: Option<Arc<ToolDescriptorRegistry>>,
        context_view: ContextViewSettings,
        config: Arc<AppConfig>,
        identity: tokio::sync::watch::Receiver<Arc<str>>,
        promotion_suppressed_during_step: Arc<AtomicBool>,
    ) -> Self {
        Self {
            state: AgentState::Idle,
            engine,
            gatekeeper,
            ephemeral,
            context_assembler: ContextAssembler::new(
                vault_root,
                workspace,
                identity,
                config.staged_memory_prompt_max_chars,
            )
            .with_grammar_constraint(config.is_llamacpp()),
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
            presentation_tx,
            queued_inputs: 0,
            last_router_ms: 0,
            last_llm_ms: 0,
            last_tool_ms: 0,
            last_total_ms: 0,
            last_top_tool_match: None,
            last_turn_tools_enabled: false,
            descriptor_jit_top_k,
            descriptor_jit_max_chars,
            slim_tool_prompt,
            tool_map_offer_cap,
            descriptor_registry,
            context_view,
            config,
            force_full_tool_schemas_in_llm_view: false,
            turn_seq: 0,
            activity_line: None,
            last_deck_message_body: None,
            tool_round_cap_final_pass_pending: false,
            promotion_suppressed_during_step,
            moltbook_browse_ledger: None,
            tool_repeat_failure_streak: HashMap::new(),
            step_failed_tools: HashSet::new(),
        }
    }

    pub(super) fn llm_view_settings(&self) -> ContextViewSettings {
        let mut s = self.context_view.clone();
        s.full_tool_schemas_in_llm_view =
            s.full_tool_schemas_in_llm_view || self.force_full_tool_schemas_in_llm_view;
        s
    }
}
