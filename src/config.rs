use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::tools::ToolContextViewHint;

/// Optional Google Workspace (Gmail API) credentials. When `enabled`, both paths must be set.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct GoogleConfig {
    /// When true, Gmail tools may run; requires `service_account_key` and `impersonate_user`.
    #[serde(default)]
    pub enabled: bool,
    /// Path to a service-account JSON key (often `.fcp/eris-sa.json` under the vault).
    pub service_account_key: Option<PathBuf>,
    /// Workspace user email to impersonate (domain-wide delegation).
    pub impersonate_user: Option<String>,
}

/// HTTP API profile for [`crate::util::ApiHttpClient`] (URL/query/header templates). Map keys are profile ids (`[apis.<id>]` in `.fcp/config.toml`).
/// One HTTP template profile for [`crate::util::ApiHttpClient`] (`[apis.<id>]` in TOML).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ApiProfile {
    /// When false, tools that need this profile skip remote calls.
    #[serde(default = "default_api_profile_enabled")]
    pub enabled: bool,
    /// URL with `{placeholder}` segments replaced at call time.
    pub base_url: String,
    /// Query string key/value templates (placeholders in values).
    #[serde(default)]
    pub query: HashMap<String, String>,
    /// Extra headers (e.g. Wikipedia `User-Agent`).
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Response size cap; when unset, [`AppConfig::web_fetch_max_bytes`] applies.
    #[serde(default)]
    pub max_response_bytes: Option<usize>,
    /// Optional HTTP cache TTL for profiles that support it.
    #[serde(default)]
    pub stale_after_secs: Option<u64>,
}

/// Built-in weather/wiki API profiles default to enabled.
fn default_api_profile_enabled() -> bool {
    true
}

/// Curated vault paths relative to chat `workspace_root`. Identity file paths trigger snapshot hot-reload; `99_USER_UPLOADED` is watched recursively (activity is logged, see `spawn_vault_identity_watch`).
/// Debounced `notify` paths under the chat workspace root (identity hot-reload, uploads, etc.).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct VaultWatchConfig {
    /// Master switch for the vault file watcher in chat startup.
    #[serde(default = "default_vault_watch_enabled")]
    pub enabled: bool,
    /// Coalesce rapid writes before reloading identity snapshot.
    #[serde(default = "default_vault_watch_debounce_ms")]
    pub debounce_ms: u64,
    /// Paths relative to vault root; `99_USER_UPLOADED` is watched recursively where supported.
    #[serde(default = "default_vault_watch_paths")]
    pub paths: Vec<String>,
}

fn default_vault_watch_enabled() -> bool {
    true
}

fn default_vault_watch_debounce_ms() -> u64 {
    120
}

fn default_vault_watch_paths() -> Vec<String> {
    vec![
        "00_Invariants/Identity.md".to_string(),
        "99_USER_UPLOADED".to_string(),
    ]
}

impl Default for VaultWatchConfig {
    fn default() -> Self {
        Self {
            enabled: default_vault_watch_enabled(),
            debounce_ms: default_vault_watch_debounce_ms(),
            paths: default_vault_watch_paths(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct AppConfig {
    /// Logical workspace id: Qdrant collection `fcp_vault_v2_{workspace}`, ephemeral file suffix, etc.
    /// Chat does **not** use `vault_root`/`workspace` as the on-disk vault; see [`Self::active_vault`].
    pub workspace: String,
    /// Legacy / CLI override path; chat uses [`Self::config_source_dir`] (cwd at load) as the vault.
    pub vault_root: PathBuf,
    /// `tracing` filter directive (e.g. `info`, `debug`); also read as `FCP_LOG_LEVEL`.
    pub log_level: String,
    /// Ollama HTTP base URL (no trailing path), e.g. `http://localhost:11434`.
    pub ollama_host: String,
    /// Default chat model id as understood by Ollama (`ollama pull …`).
    pub model_name: String,
    /// Operator display name for UI / prompts; from TOML or `FCP_USER_NAME`; empty if unset.
    #[serde(default)]
    pub user_name: String,
    /// Context window size passed to Ollama as `num_ctx` / generation options.
    pub num_ctx: usize,
    /// Max seconds to wait for a single LLM generation (connect + stream).
    pub generation_timeout_secs: u64,
    /// Forwarded to Ollama on each chat request as `.think(...)` in `OllamaClient::generate` (`ollama-rs` `ChatMessageRequest`). `false` (default) turns off the separate thinking/reasoning channel for models that support it—saves tokens and RAM versus `true`. TOML key name is historical; unrelated to `engine::router::ReasoningRouter`.
    pub enable_reasoning_fsm: bool,
    /// Fraction of estimated context fill (0.0–1.0) at which rolling condensation runs.
    pub condensation_threshold: f32,
    /// Target token budget after condensation (rolling summary + retained tail).
    pub condensation_target: usize,
    /// Max tool-call rounds per user `step()` before cap recovery / final pass.
    pub max_tool_rounds: u8,
    /// Max schema/recovery retries before the orchestrator bails to idle.
    pub max_recovery_attempts: u8,
    /// TTL for `EphemeralTier::Session` rows (seconds). Shortest-lived tier.
    #[serde(default = "default_ephemeral_ttl_session_secs")]
    pub ephemeral_ttl_session_secs: u64,
    /// TTL for `EphemeralTier::Scratch` rows (seconds). Working-note tier.
    #[serde(default = "default_ephemeral_ttl_scratch_secs")]
    pub ephemeral_ttl_scratch_secs: u64,
    /// TTL for `EphemeralTier::Promote` rows (seconds). Longest ephemeral tier; commit-eligible.
    #[serde(default = "default_ephemeral_ttl_promote_secs")]
    pub ephemeral_ttl_promote_secs: u64,
    /// Score threshold for session -> scratch promotion.
    #[serde(default = "default_promotion_threshold_session_to_scratch")]
    pub promotion_threshold_session_to_scratch: f64,
    /// Score threshold for scratch -> promote promotion.
    #[serde(default = "default_promotion_threshold_scratch_to_promote")]
    pub promotion_threshold_scratch_to_promote: f64,
    /// Per-tick decay subtracted from `promotion_score` by the snapshot daemon.
    #[serde(default = "default_promotion_decay_per_tick")]
    pub promotion_decay_per_tick: f64,
    /// Interval (seconds) at which the daemon evaluates tier transitions and decay.
    #[serde(default = "default_promotion_eval_interval_secs")]
    pub promotion_eval_interval_secs: u64,
    /// Score boost applied per distinct turn a `canonical_key` appears.
    #[serde(default = "default_promotion_mention_boost")]
    pub promotion_mention_boost: f64,
    /// Score boost for explicit `memory:stage` calls.
    #[serde(default = "default_promotion_stage_boost")]
    pub promotion_stage_boost: f64,
    /// When true, after each completed assistant turn the runtime matches user text against staged `canonical_key` tokens and bumps score + refreshes TTL.
    #[serde(default = "default_turn_end_mention_enabled")]
    pub turn_end_mention_enabled: bool,
    /// Max characters for the `[ACTIVE_STAGED_MEMORY]` block injected into system prompts; `0` disables.
    #[serde(default = "default_staged_memory_prompt_max_chars")]
    pub staged_memory_prompt_max_chars: usize,
    /// When true, `web:fetch` is stripped from tool allowlists (deprecated in favor of other flows).
    #[serde(default)]
    pub web_fetch_deprecated: bool,
    /// Qdrant gRPC endpoint URL (semantic memory / `memory:query`).
    pub qdrant_url: String,
    /// Qdrant collection name. Computed at runtime: `fcp_vault_v2_{workspace}`.
    #[serde(skip)]
    pub qdrant_collection_v2: String,
    /// How often the ephemeral daemon writes `.fcp/ephemeral_{workspace}.bin` and purges expiry.
    pub snapshot_interval_secs: u64,
    /// Ollama embedding model for ToolRouter and Qdrant upserts (vector width must match collection).
    pub embed_model_name: String,
    /// Seconds without user input before heartbeat may treat the session as idle (if heartbeat enabled).
    pub idle_timeout_secs: u64,
    /// When true, spawn the idle monitor that may inject [`crate::executive::error::FcpError::Interrupted`] after [`Self::idle_timeout_secs`]. Default off until Gardener/sleep features land.
    #[serde(default)]
    pub idle_heartbeat_enabled: bool,
    /// HTTP timeout for `web:fetch` tool requests.
    pub web_fetch_timeout_secs: u64,
    /// Default max response body size for web fetch and API profiles without their own cap.
    pub web_fetch_max_bytes: usize,
    /// Fraction of [`Self::num_ctx`] used to cap vault read and web chunk sizes in tools (not the condensation trigger).
    pub vault_read_ratio: f32,
    /// TTL for ephemeral **buffer** rows (`vault:read` large files, `web:fetch` artifacts, `ephemeral:buffer_page`).
    #[serde(default = "default_ephemeral_buffer_ttl_secs")]
    pub ephemeral_buffer_ttl_secs: u64,
    /// Max chunk count for a single staged buffer (stage fails if exceeded).
    #[serde(default = "default_ephemeral_buffer_max_chunks")]
    pub ephemeral_buffer_max_chunks: usize,
    /// Cosine similarity floor for ToolRouter pre-LLM semantic matches (0.0–1.0).
    pub tool_match_threshold: f32,
    /// Number of semantic-router hits that receive extra “when to use” descriptor text in tool mode.
    #[serde(default = "default_tool_descriptor_jit_top_k")]
    pub tool_descriptor_jit_top_k: usize,
    /// Character budget for the JIT descriptor block appended to the system prompt.
    #[serde(default = "default_tool_descriptor_jit_max_chars")]
    pub tool_descriptor_jit_max_chars: usize,
    /// When true, tool-mode prompts use a generated phrase map plus tool defs without `parameters` (smaller context). Full JSON Schema is supplied on gatekeeper schema recovery.
    #[serde(default = "default_slim_tool_prompt")]
    pub slim_tool_prompt: bool,
    /// When [`Self::slim_tool_prompt`] is true and the semantic router returned hits, include at most this many tools (in router order). `0` means no cap (use full hit list). Ignored when the router returns no hits (full allowed roster, still slim).
    #[serde(default = "default_tool_map_offer_cap")]
    pub tool_map_offer_cap: usize,
    /// Command used when chat startup asks to launch a local Ollama if unreachable.
    #[serde(default = "default_ollama_daemon")]
    pub ollama_daemon: DaemonCommand,
    /// Command used when chat startup asks to launch Qdrant if unreachable.
    #[serde(default = "default_qdrant_daemon")]
    pub qdrant_daemon: DaemonCommand,
    /// When true, startup fails if Qdrant gRPC (semantic brain) cannot connect after retries.
    #[serde(default = "default_require_semantic_brain")]
    pub require_semantic_brain: bool,
    /// Max attempts for `SemanticBrain::new` (gRPC to Qdrant), including the first try.
    #[serde(default = "default_semantic_brain_connect_attempts")]
    pub semantic_brain_connect_attempts: u32,
    /// Delay between failed gRPC connect attempts (milliseconds).
    #[serde(default = "default_semantic_brain_connect_retry_delay_ms")]
    pub semantic_brain_connect_retry_delay_ms: u64,
    /// Named HTTP profiles for weather, wiki, etc.; merged over [`default_builtin_apis`] at default time, TOML overrides by key.
    #[serde(default)]
    pub apis: HashMap<String, ApiProfile>,
    /// Debounced filesystem watch for identity and uploads under the vault (see [`VaultWatchConfig`]).
    #[serde(default)]
    pub vault_watch: VaultWatchConfig,
    /// When true, [`crate::orchestrator::context::build_llm_view`] feeds a lean copy to the LLM only; [`crate::orchestrator::core::Orchestrator::chat_stack`] stays full fidelity.
    #[serde(default = "default_optimize_context")]
    pub optimize_context: bool,
    /// Default max chars for tool result bodies in the LLM view when a tool uses [`ToolContextViewHint::Default`].
    #[serde(default = "default_optimize_context_max_tool_snippet_chars")]
    pub optimize_context_max_tool_snippet_chars: usize,
    /// Strip assistant JSON to `message_to_user` + tool names in the LLM view.
    #[serde(default = "default_optimize_context_assistant_compact")]
    pub optimize_context_assistant_compact: bool,
    /// Optional per-tool overrides (merged on top of each tool’s `context_view_hint()`).
    #[serde(default)]
    pub optimize_context_tool_overrides: HashMap<String, ToolContextViewHint>,
    /// Gmail / Google Workspace integration (service account + domain-wide delegation).
    #[serde(default)]
    pub google: GoogleConfig,
    /// When true, keep full JSON parameter schemas in the LLM view for tool definitions (larger prompt). When false and [`Self::optimize_context`] is true, [`crate::orchestrator::context::build_llm_view`] strips `parameters` in that block only; [`crate::orchestrator::core::Orchestrator::chat_stack`] stays full. Independently, the orchestrator forces full schemas for one recovery LLM pass after a Gatekeeper schema fault ([`crate::orchestrator::core::Orchestrator::force_full_tool_schemas_in_llm_view`]).
    #[serde(default = "default_optimize_context_full_tool_schemas")]
    pub optimize_context_full_tool_schemas: bool,
    /// When true and [`Self::optimize_context`] is true, collapse resolved tool-recovery spans in the LLM view only (canonical [`crate::orchestrator::core::Orchestrator::chat_stack`] unchanged).
    #[serde(default = "default_optimize_context_omit_resolved_tool_recovery")]
    pub optimize_context_omit_resolved_tool_recovery: bool,
    /// Default `top_k` for [`crate::tools::memory::MemoryQueryTool`] when the LLM omits it.
    #[serde(default = "default_memory_query_default_top_k")]
    pub memory_query_default_top_k: u32,
    /// Upper bound for `top_k` in `memory:query` (clamps user/LLM input).
    #[serde(default = "default_memory_query_top_k_max")]
    pub memory_query_top_k_max: u32,
    /// Default total character budget for formatted `memory:query` output.
    #[serde(default = "default_memory_query_default_max_total_chars")]
    pub memory_query_default_max_total_chars: u32,
    /// Minimum allowed `max_total_chars` when the caller passes a value (floor).
    #[serde(default = "default_memory_query_min_max_total_chars")]
    pub memory_query_min_max_total_chars: u32,
    /// Max Qdrant points to retrieve when post-filtering (e.g. `vault_path_prefix`) needs headroom.
    #[serde(default = "default_memory_query_oversample_cap")]
    pub memory_query_oversample_cap: u64,
    /// Multiplier for oversampling when a path prefix filter is active (`top_k * multiplier`, then capped).
    #[serde(default = "default_memory_query_oversample_multiplier")]
    pub memory_query_oversample_multiplier: u64,
    /// Minimum Qdrant limit when oversampling for path prefix.
    #[serde(default = "default_memory_query_oversample_min")]
    pub memory_query_oversample_min: u64,
    /// Bind address for [`crate::ui::web::run_web_chat`] (`eris chat --web`). Prefer loopback until bind/auth are hardened.
    #[serde(default = "default_web_bind_addr")]
    pub web_bind_addr: String,
    /// TCP port for the web chat server.
    #[serde(default = "default_web_port")]
    pub web_port: u16,
    /// When true, `eris chat --web` opens the listen URL in the system default browser after bind. Set `false` for SSH/headless.
    #[serde(default = "default_web_open_browser")]
    pub web_open_browser: bool,
    /// Current working directory when [`AppConfig::load`] ran — this is the physical vault root for chat.
    #[serde(skip)]
    pub config_source_dir: PathBuf,
}

/// Spawnable daemon for peripheral bootstrap (`[ollama_daemon]` / `[qdrant_daemon]`).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DaemonCommand {
    /// Executable name on `PATH` or absolute path.
    pub command: String,
    /// argv after the program name (may be empty).
    #[serde(default)]
    pub args: Vec<String>,
}

/// Default peripheral spawn: `ollama serve`.
fn default_ollama_daemon() -> DaemonCommand {
    DaemonCommand {
        command: "ollama".into(),
        args: vec!["serve".into()],
    }
}

/// Default peripheral spawn: `qdrant` with no args (container/binary default config).
fn default_qdrant_daemon() -> DaemonCommand {
    DaemonCommand {
        command: "qdrant".into(),
        args: Vec::new(),
    }
}

/// How many top semantic router hits get extra JIT descriptor text in the system prompt.
fn default_tool_descriptor_jit_top_k() -> usize {
    3
}

/// Character budget for that JIT descriptor block.
fn default_tool_descriptor_jit_max_chars() -> usize {
    6000
}

fn default_slim_tool_prompt() -> bool {
    true
}

/// With slim tool prompt + semantic hits, cap how many matched tools appear in the phrase map; `0` = no cap.
fn default_tool_map_offer_cap() -> usize {
    0
}

/// When true, chat startup fails if Qdrant is unreachable after retries.
fn default_require_semantic_brain() -> bool {
    true
}

/// gRPC connect attempts to Qdrant (including the first try).
fn default_semantic_brain_connect_attempts() -> u32 {
    12
}

/// Backoff between failed Qdrant connect attempts (ms).
fn default_semantic_brain_connect_retry_delay_ms() -> u64 {
    500
}

/// TTL for staged big buffers (vault large read, web artifact); default ~10 minutes.
fn default_ephemeral_buffer_ttl_secs() -> u64 {
    600
}

/// Upper bound on chunk count per buffer to bound memory and snapshot size.
fn default_ephemeral_buffer_max_chunks() -> usize {
    4096
}

/// TTL for `EphemeralTier::Session` (~15 minutes). Idle expiry; turn-end mention can refresh TTL.
fn default_ephemeral_ttl_session_secs() -> u64 {
    900
}

/// TTL for `EphemeralTier::Scratch` (working notes between session and commit review).
fn default_ephemeral_ttl_scratch_secs() -> u64 {
    3600
}

/// TTL for `EphemeralTier::Promote` (commit-eligible; long session before user runs commit).
fn default_ephemeral_ttl_promote_secs() -> u64 {
    28800
}

/// Minimum `promotion_score` to move from Session → Scratch.
fn default_promotion_threshold_session_to_scratch() -> f64 {
    3.0
}

/// Minimum `promotion_score` to move from Scratch → Promote.
fn default_promotion_threshold_scratch_to_promote() -> f64 {
    6.0
}

/// Subtracted from `promotion_score` on each promotion daemon tick (when not suppressed).
fn default_promotion_decay_per_tick() -> f64 {
    0.5
}

/// How often the daemon runs `evaluate_promotions_and_decay` (skipped while `Orchestrator::step` is active).
fn default_promotion_eval_interval_secs() -> u64 {
    120
}

/// Score added per distinct user turn that references a staged entry’s `canonical_key`.
fn default_promotion_mention_boost() -> f64 {
    1.0
}

/// Score added on explicit `memory:stage`; with default decay (0.5) and eval interval, one tick can cross session→scratch (3.0).
fn default_promotion_stage_boost() -> f64 {
    3.5
}

/// Bump promotion score / TTL when user text mentions a staged `canonical_key` after a turn.
fn default_turn_end_mention_enabled() -> bool {
    true
}

/// Max size of the `[ACTIVE_STAGED_MEMORY]` block in the system prompt; `0` disables.
fn default_staged_memory_prompt_max_chars() -> usize {
    1500
}

/// When true, build a slimmer copy of history for the LLM via [`crate::orchestrator::context::build_llm_view`].
fn default_optimize_context() -> bool {
    true
}

/// Default max chars per tool result body in the LLM view; align with orchestrator caps to avoid double truncation.
fn default_optimize_context_max_tool_snippet_chars() -> usize {
    2500
}

/// When optimizing context, shrink assistant JSON in the LLM view to `message_to_user` + tool names.
fn default_optimize_context_assistant_compact() -> bool {
    true
}

/// When true and `optimize_context`, keep full JSON Schema `parameters` in the tool-def block (larger prompts).
fn default_optimize_context_full_tool_schemas() -> bool {
    false
}

fn default_optimize_context_omit_resolved_tool_recovery() -> bool {
    true
}

/// Default `top_k` for `memory:query` when the model omits it.
fn default_memory_query_default_top_k() -> u32 {
    5
}

/// Hard cap for `top_k` (clamps LLM/user args).
fn default_memory_query_top_k_max() -> u32 {
    25
}

/// Default total character budget for formatted vector hits returned to the model.
fn default_memory_query_default_max_total_chars() -> u32 {
    10_000
}

/// Floor for `max_total_chars` when the caller supplies a value.
fn default_memory_query_min_max_total_chars() -> u32 {
    256
}

/// Max Qdrant points to pull when post-filtering (e.g. `vault_path_prefix`) needs headroom.
fn default_memory_query_oversample_cap() -> u64 {
    200
}

/// Oversample limit = `top_k * multiplier` before cap/min when a path prefix filter is active.
fn default_memory_query_oversample_multiplier() -> u64 {
    25
}

/// Minimum Qdrant limit during oversampling for path-prefix queries.
fn default_memory_query_oversample_min() -> u64 {
    30
}

/// Built-in Open-Meteo (non-commercial) API profiles for [`crate::tools::weather`]. Override or extend via `.fcp/config.toml` `[apis.*]`.
pub fn default_open_meteo_apis() -> HashMap<String, ApiProfile> {
    let mut m = HashMap::new();
    m.insert(
        "open_meteo_geocode".into(),
        ApiProfile {
            enabled: true,
            base_url: "https://geocoding-api.open-meteo.com/v1/search".into(),
            query: [
                ("name".into(), "{city}".into()),
                ("count".into(), "1".into()),
            ]
            .into_iter()
            .collect(),
            headers: HashMap::new(),
            max_response_bytes: Some(65_536),
            stale_after_secs: None,
        },
    );
    m.insert(
        "open_meteo_geocode_cc".into(),
        ApiProfile {
            enabled: true,
            base_url: "https://geocoding-api.open-meteo.com/v1/search".into(),
            query: [
                ("name".into(), "{city}".into()),
                ("count".into(), "1".into()),
                ("countryCode".into(), "{country_code}".into()),
            ]
            .into_iter()
            .collect(),
            headers: HashMap::new(),
            max_response_bytes: Some(65_536),
            stale_after_secs: None,
        },
    );
    m.insert(
        "open_meteo_forecast_current".into(),
        ApiProfile {
            enabled: true,
            base_url: "https://api.open-meteo.com/v1/forecast".into(),
            query: [
                ("latitude".into(), "{lat}".into()),
                ("longitude".into(), "{lon}".into()),
                (
                    "current".into(),
                    "temperature_2m,weather_code,relative_humidity_2m,precipitation,cloud_cover"
                        .into(),
                ),
                ("timezone".into(), "auto".into()),
            ]
            .into_iter()
            .collect(),
            headers: HashMap::new(),
            max_response_bytes: None,
            stale_after_secs: None,
        },
    );
    m.insert(
        "open_meteo_forecast_hourly".into(),
        ApiProfile {
            enabled: true,
            base_url: "https://api.open-meteo.com/v1/forecast".into(),
            query: [
                ("latitude".into(), "{lat}".into()),
                ("longitude".into(), "{lon}".into()),
                (
                    "hourly".into(),
                    "temperature_2m,precipitation,cloud_cover".into(),
                ),
                ("forecast_days".into(), "3".into()),
                ("timezone".into(), "auto".into()),
            ]
            .into_iter()
            .collect(),
            headers: HashMap::new(),
            max_response_bytes: None,
            stale_after_secs: None,
        },
    );
    m
}

/// English Wikipedia REST summary profile for [`crate::tools::wiki::WikiSummaryTool`]. Wikimedia requires a descriptive User-Agent.
pub fn default_wikipedia_page_summary_api() -> HashMap<String, ApiProfile> {
    let mut headers = HashMap::new();
    headers.insert(
        "User-Agent".into(),
        "Eris-Agent/1.0 (Local autonomous system)".into(),
    );
    let mut m = HashMap::new();
    m.insert(
        "wikipedia_page_summary".into(),
        ApiProfile {
            enabled: true,
            base_url: "https://en.wikipedia.org/api/rest_v1/page/summary/{title}".into(),
            query: HashMap::new(),
            headers,
            max_response_bytes: Some(65_536),
            stale_after_secs: None,
        },
    );
    m
}

/// Weather + Wikipedia profiles merged into [`AppConfig::default`]; TOML `[apis]` entries override by id.
fn default_builtin_apis() -> HashMap<String, ApiProfile> {
    let mut apis = default_open_meteo_apis();
    apis.extend(default_wikipedia_page_summary_api());
    apis
}

fn default_web_bind_addr() -> String {
    "127.0.0.1".into()
}

fn default_web_port() -> u16 {
    8787
}

fn default_web_open_browser() -> bool {
    true
}

impl Default for AppConfig {
    /// Baseline profile aligned with a typical local Mac setup (Ollama + Qdrant); override per vault in `.fcp/config.toml` or `FCP_*`.
    /// A checked-in full example (including optional Gmail) is `vaults/nemo/.fcp/config.toml` — copy and trim for new vaults.
    fn default() -> Self {
        Self {
            workspace: "default".into(),
            vault_root: PathBuf::from("./vaults/"),
            log_level: "info".into(),
            ollama_host: "http://localhost:11434".into(),
            model_name: "gemma4:26b".into(),
            user_name: String::new(),
            num_ctx: 16384,
            generation_timeout_secs: 120,
            enable_reasoning_fsm: false,
            condensation_threshold: 0.5,
            condensation_target: 300,
            max_tool_rounds: 5,
            max_recovery_attempts: 3,
            ephemeral_ttl_session_secs: default_ephemeral_ttl_session_secs(),
            ephemeral_ttl_scratch_secs: default_ephemeral_ttl_scratch_secs(),
            ephemeral_ttl_promote_secs: default_ephemeral_ttl_promote_secs(),
            promotion_threshold_session_to_scratch: default_promotion_threshold_session_to_scratch(),
            promotion_threshold_scratch_to_promote: default_promotion_threshold_scratch_to_promote(),
            promotion_decay_per_tick: default_promotion_decay_per_tick(),
            promotion_eval_interval_secs: default_promotion_eval_interval_secs(),
            promotion_mention_boost: default_promotion_mention_boost(),
            promotion_stage_boost: default_promotion_stage_boost(),
            turn_end_mention_enabled: default_turn_end_mention_enabled(),
            staged_memory_prompt_max_chars: default_staged_memory_prompt_max_chars(),
            web_fetch_deprecated: false,
            qdrant_url: "http://localhost:6334".into(),
            qdrant_collection_v2: "fcp_vault_v2_default".into(),
            snapshot_interval_secs: 300,
            embed_model_name: "nomic-embed-text".into(),
            idle_timeout_secs: 900,
            idle_heartbeat_enabled: false,
            web_fetch_timeout_secs: 10,
            web_fetch_max_bytes: 20480,
            vault_read_ratio: 0.5,
            ephemeral_buffer_ttl_secs: default_ephemeral_buffer_ttl_secs(),
            ephemeral_buffer_max_chunks: default_ephemeral_buffer_max_chunks(),
            tool_match_threshold: 0.50,
            tool_descriptor_jit_top_k: default_tool_descriptor_jit_top_k(),
            tool_descriptor_jit_max_chars: default_tool_descriptor_jit_max_chars(),
            slim_tool_prompt: default_slim_tool_prompt(),
            tool_map_offer_cap: default_tool_map_offer_cap(),
            ollama_daemon: default_ollama_daemon(),
            qdrant_daemon: default_qdrant_daemon(),
            require_semantic_brain: default_require_semantic_brain(),
            semantic_brain_connect_attempts: default_semantic_brain_connect_attempts(),
            semantic_brain_connect_retry_delay_ms: default_semantic_brain_connect_retry_delay_ms(),
            apis: default_builtin_apis(),
            vault_watch: VaultWatchConfig::default(),
            optimize_context: default_optimize_context(),
            optimize_context_max_tool_snippet_chars: default_optimize_context_max_tool_snippet_chars(),
            optimize_context_assistant_compact: default_optimize_context_assistant_compact(),
            optimize_context_tool_overrides: HashMap::new(),
            google: GoogleConfig::default(),

            optimize_context_full_tool_schemas: default_optimize_context_full_tool_schemas(),
            optimize_context_omit_resolved_tool_recovery:
                default_optimize_context_omit_resolved_tool_recovery(),
            memory_query_default_top_k: default_memory_query_default_top_k(),
            memory_query_top_k_max: default_memory_query_top_k_max(),
            memory_query_default_max_total_chars: default_memory_query_default_max_total_chars(),
            memory_query_min_max_total_chars: default_memory_query_min_max_total_chars(),
            memory_query_oversample_cap: default_memory_query_oversample_cap(),
            memory_query_oversample_multiplier: default_memory_query_oversample_multiplier(),
            memory_query_oversample_min: default_memory_query_oversample_min(),
            web_bind_addr: default_web_bind_addr(),
            web_port: default_web_port(),
            web_open_browser: default_web_open_browser(),
            config_source_dir: PathBuf::new(),
        }
    }
}

impl AppConfig {
    pub fn load(cli: crate::executive::cli::Cli) -> crate::executive::error::Result<Self> {
        use figment::{Figment, providers::{Env, Format, Toml}};

        let _ = dotenvy::dotenv();

        let figment = Figment::from(figment::providers::Serialized::defaults(AppConfig::default()))
            .merge(Toml::file(crate::vault_layout::config_toml(std::path::Path::new("."))))
            .merge(Env::prefixed("FCP_"));

        let mut config: AppConfig = figment.extract().map_err(|e| crate::executive::error::FcpError::Config(e.to_string()))?;

        config.config_source_dir = std::env::current_dir().map_err(|e| {
            crate::executive::error::FcpError::Config(format!("Could not read current directory: {}", e))
        })?;

        if cli.workspace != "default" {
            config.workspace = cli.workspace;
        }

        if let Some(vault) = cli.vault {
            config.vault_root = vault;
        }

        config.qdrant_collection_v2 = format!("fcp_vault_v2_{}", config.workspace);

        Ok(config)
    }

    /// TTL in seconds for the given ephemeral tier.
    pub fn ttl_for_tier(&self, tier: crate::memory::types::EphemeralTier) -> u64 {
        match tier {
            crate::memory::types::EphemeralTier::Session => self.ephemeral_ttl_session_secs,
            crate::memory::types::EphemeralTier::Scratch => self.ephemeral_ttl_scratch_secs,
            crate::memory::types::EphemeralTier::Promote => self.ephemeral_ttl_promote_secs,
        }
    }

    /// Score threshold required to promote *from* the given tier to the next.
    /// Returns `None` for `Promote` (no next tier).
    pub fn promotion_threshold_for_tier(&self, tier: crate::memory::types::EphemeralTier) -> Option<f64> {
        match tier {
            crate::memory::types::EphemeralTier::Session => Some(self.promotion_threshold_session_to_scratch),
            crate::memory::types::EphemeralTier::Scratch => Some(self.promotion_threshold_scratch_to_promote),
            crate::memory::types::EphemeralTier::Promote => None,
        }
    }

    /// Physical directory for chat, ignition, tools, and `.fcp/` — always the process working directory
    /// at [`AppConfig::load`] (i.e. `cd` into your vault, then run `eris chat`).
    ///
    /// [`Self::workspace`] and [`Self::vault_root`] do **not** form this path; they remain logical / legacy config only.
    pub fn active_vault(&self) -> PathBuf {
        self.config_source_dir.clone()
    }

    /// Absolute paths under `chat_workspace_root` for vault watch (e.g. `00_Invariants/Identity.md`).
    pub fn resolved_vault_watch_file_paths(&self, chat_workspace_root: &std::path::Path) -> Vec<std::path::PathBuf> {
        self.vault_watch
            .paths
            .iter()
            .map(|rel| chat_workspace_root.join(rel))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::executive::cli::{Cli, Commands};
    use std::path::PathBuf;

    #[test]
    fn test_config_hierarchy_and_dynamic_resolution() {
        figment::Jail::expect_with(|jail| {
            jail.create_dir(".fcp")?;
            jail.create_file(".fcp/config.toml", r#"
                workspace = "toml_workspace"
                vault_root = "/toml/vaults"
                log_level = "warn"
            "#)?;

            jail.create_file(".env", r#"
                FCP_WORKSPACE=env_workspace
                FCP_LOG_LEVEL=error
            "#)?;

            jail.set_env("FCP_WORKSPACE", "env_workspace");
            jail.set_env("FCP_LOG_LEVEL", "error");

            let cli = Cli {
                workspace: "cli_workspace".to_string(),
                vault: Some(PathBuf::from("/cli/vaults")),
                verbose: 0,
                command: Commands::Chat { web: false },
            };

            let config = AppConfig::load(cli).expect("Failed to load config");

            assert_eq!(config.workspace, "cli_workspace");
            assert_eq!(config.vault_root, PathBuf::from("/cli/vaults"));
            assert_eq!(config.log_level, "error");
            assert_eq!(config.qdrant_collection_v2, "fcp_vault_v2_cli_workspace");

            // Test fallback
            let cli2 = Cli {
                workspace: "default".to_string(),
                vault: None,
                verbose: 0,
                command: Commands::Chat { web: false },
            };

            let config2 = AppConfig::load(cli2).expect("Failed to load config");

            assert_eq!(config2.workspace, "env_workspace");
            assert_eq!(config2.vault_root, PathBuf::from("/toml/vaults"));
            assert_eq!(config2.log_level, "error");
            assert_eq!(config2.qdrant_collection_v2, "fcp_vault_v2_env_workspace");

            Ok(())
        });
    }

    #[test]
    fn test_app_config_is_pure_data() {
        let json_data = r#"{
            "workspace": "test_workspace",
            "vault_root": "/tmp/vaults",
            "log_level": "debug",
            "ollama_host": "http://localhost:11434",
            "model_name": "qwen2.5:14b",
            "user_name": "",
            "num_ctx": 32768,
            "generation_timeout_secs": 60,
            "enable_reasoning_fsm": false,
            "condensation_threshold": 0.5,
            "condensation_target": 500,
            "max_tool_rounds": 10,
            "max_recovery_attempts": 5,
            "qdrant_url": "http://localhost:6334",
            "snapshot_interval_secs": 600,
            "embed_model_name": "nomic-embed-text",
            "idle_timeout_secs": 42,
            "web_fetch_timeout_secs": 15,
            "web_fetch_max_bytes": 10240,
            "vault_read_ratio": 0.25,
            "tool_match_threshold": 0.50,
            "ollama_daemon": { "command": "ollama", "args": ["serve"] },
            "qdrant_daemon": { "command": "qdrant", "args": [] }
        }"#;

        let parsed_config: AppConfig = serde_json::from_str(json_data).expect("Failed to parse JSON");

        assert_eq!(parsed_config.workspace, "test_workspace");
        assert_eq!(parsed_config.vault_root, PathBuf::from("/tmp/vaults"));
        assert_eq!(parsed_config.log_level, "debug");
        assert_eq!(parsed_config.ollama_host, "http://localhost:11434");
        assert_eq!(parsed_config.model_name, "qwen2.5:14b");
        assert_eq!(parsed_config.user_name, "");
        assert_eq!(parsed_config.num_ctx, 32768);
        assert_eq!(parsed_config.generation_timeout_secs, 60);
        assert_eq!(parsed_config.enable_reasoning_fsm, false);
        assert_eq!(parsed_config.condensation_threshold, 0.5);
        assert_eq!(parsed_config.condensation_target, 500);
        assert_eq!(parsed_config.max_tool_rounds, 10);
        assert_eq!(parsed_config.max_recovery_attempts, 5);
        assert_eq!(parsed_config.qdrant_url, "http://localhost:6334");
        assert_eq!(parsed_config.snapshot_interval_secs, 600);
        assert_eq!(parsed_config.embed_model_name, "nomic-embed-text");
        assert_eq!(parsed_config.idle_timeout_secs, 42);
        assert_eq!(parsed_config.web_fetch_timeout_secs, 15);
        assert_eq!(parsed_config.web_fetch_max_bytes, 10240);
        assert_eq!(parsed_config.vault_read_ratio, 0.25);
        assert_eq!(parsed_config.tool_match_threshold, 0.50);
        assert_eq!(parsed_config.ollama_daemon.command, "ollama");
        assert_eq!(parsed_config.ollama_daemon.args, vec!["serve"]);
        assert_eq!(parsed_config.qdrant_daemon.command, "qdrant");
        assert!(parsed_config.qdrant_daemon.args.is_empty());
        assert!(parsed_config.apis.is_empty());
    }

    #[test]
    fn active_vault_is_always_config_source_dir() {
        let mut c = AppConfig::default();
        c.config_source_dir = PathBuf::from("/any/adam");
        c.workspace = "something_else".into();
        c.vault_root = PathBuf::from("./vaults/");
        assert_eq!(c.active_vault(), PathBuf::from("/any/adam"));
    }

    #[test]
    fn default_config_includes_open_meteo_api_profiles() {
        let c = AppConfig::default();
        assert_eq!(c.apis.len(), 5);
        assert!(c.apis.contains_key("open_meteo_geocode"));
        assert!(c.apis.contains_key("open_meteo_geocode_cc"));
        assert!(c.apis.contains_key("open_meteo_forecast_current"));
        assert!(c.apis.contains_key("open_meteo_forecast_hourly"));
        assert!(c.apis.contains_key("wikipedia_page_summary"));
        let wiki = c.apis.get("wikipedia_page_summary").expect("wiki profile");
        assert!(wiki.headers.contains_key("User-Agent"));
    }

    #[test]
    fn default_config_has_v2_collection_name() {
        let c = AppConfig::default();
        assert_eq!(c.qdrant_collection_v2, "fcp_vault_v2_default");
    }

    #[test]
    fn ttl_for_tier_returns_configured_values() {
        let c = AppConfig::default();
        assert_eq!(c.ttl_for_tier(crate::memory::types::EphemeralTier::Session), 900);
        assert_eq!(c.ttl_for_tier(crate::memory::types::EphemeralTier::Scratch), 3600);
        assert_eq!(c.ttl_for_tier(crate::memory::types::EphemeralTier::Promote), 28800);
    }

    #[test]
    fn promotion_threshold_returns_none_for_promote() {
        let c = AppConfig::default();
        assert!(c.promotion_threshold_for_tier(crate::memory::types::EphemeralTier::Session).is_some());
        assert!(c.promotion_threshold_for_tier(crate::memory::types::EphemeralTier::Scratch).is_some());
        assert!(c.promotion_threshold_for_tier(crate::memory::types::EphemeralTier::Promote).is_none());
    }

    #[test]
    fn vault_watch_includes_invariants_identity() {
        let c = AppConfig::default();
        assert!(c.vault_watch.paths.iter().any(|p| p.contains("00_Invariants")));
    }

    #[test]
    fn v2_collection_computed_in_load() {
        figment::Jail::expect_with(|jail| {
            jail.create_dir(".fcp")?;
            jail.create_file(".fcp/config.toml", r#"workspace = "test_v2""#)?;

            let cli = Cli {
                workspace: "default".to_string(),
                vault: None,
                verbose: 0,
                command: Commands::Chat { web: false },
            };

            let config = AppConfig::load(cli).expect("load");
            assert_eq!(config.qdrant_collection_v2, "fcp_vault_v2_test_v2");
            Ok(())
        });
    }
}
