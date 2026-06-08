use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::tools::ToolContextViewHint;

/// Optional Discord sidecar (Serenity gateway + REST). Store `application_id` / `public_key` / channel anytime; the
/// gateway only starts when [`AppConfig::discord_sidecar_should_run`] is true (needs a non-empty [`DiscordConfig::bot_token`]).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DiscordConfig {
    /// When true, run Serenity in parallel with web or terminal chat (same `user_action_tx` / presentation stream).
    #[serde(default)]
    pub enabled: bool,
    /// Discord application snowflake (Developer Portal). Used when building the HTTP client.
    pub application_id: Option<u64>,
    /// Interactions public key (hex, from Developer Portal); reserved for future slash-command HTTP verification.
    #[serde(default)]
    pub public_key: Option<String>,
    /// Target guild text channel snowflake when known.
    pub channel_id: Option<u64>,
    /// When `channel_id` is unset, the bot resolves this exact channel name against cached guild text channels at READY.
    pub channel_name: Option<String>,
    /// Bot token from TOML (trimmed). Required when [`Self::enabled`] is true; do not commit real tokens to version control.
    #[serde(default)]
    pub bot_token: Option<String>,
    /// Capacity for assistant lines queued for Discord (`try_send` from presentation mux; overflow drops with `tracing::warn`).
    #[serde(default = "default_discord_outbound_queue_capacity")]
    pub outbound_queue_capacity: usize,
}

fn default_discord_outbound_queue_capacity() -> usize {
    64
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            application_id: None,
            public_key: None,
            channel_id: None,
            channel_name: None,
            bot_token: None,
            outbound_queue_capacity: default_discord_outbound_queue_capacity(),
        }
    }
}

/// Optional Moltbook integration for the AI-agent social network.
///
/// Credentials must come from the process environment or an operator-owned file, not from
/// checked-in TOML. Authenticated requests are pinned to the default `www.moltbook.com` API
/// origin by the Moltbook client.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct MoltbookConfig {
    /// When true, register the native `moltbook:*` tools during chat startup.
    #[serde(default)]
    pub enabled: bool,
    /// Environment variable that contains the bearer token.
    #[serde(default = "default_moltbook_api_key_env")]
    pub api_key_env: String,
    /// Optional JSON credentials file, e.g. `~/.config/moltbook/credentials.json`.
    #[serde(default)]
    pub api_key_file: Option<PathBuf>,
    /// Optional expected agent name; useful for operator-facing status messages.
    #[serde(default)]
    pub agent_name: Option<String>,
    /// API base URL. Production must remain `https://www.moltbook.com/api/v1`.
    #[serde(default = "default_moltbook_base_url")]
    pub base_url: String,
    /// HTTP timeout for Moltbook API requests (seconds). Moltbook can be slow;
    /// defaults to 30s instead of the shorter `web_fetch_timeout_secs`.
    #[serde(default = "default_moltbook_timeout_secs")]
    pub timeout_secs: u64,
    /// Max UTF-8 bytes read per Moltbook JSON response before parse.
    /// Separate from [`AppConfig::web_fetch_max_bytes`]: large `moltbook:comments` / DM payloads
    /// often exceed typical web-scrape caps (~20KiB) without being maliciously huge.
    #[serde(default = "default_moltbook_max_response_bytes")]
    pub max_response_bytes: usize,
}

fn default_moltbook_api_key_env() -> String {
    "MOLTBOOK_API_KEY".into()
}

fn default_moltbook_base_url() -> String {
    "https://www.moltbook.com/api/v1".into()
}

fn default_moltbook_timeout_secs() -> u64 {
    30
}

fn default_moltbook_max_response_bytes() -> usize {
    1_048_576
}

impl Default for MoltbookConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key_env: default_moltbook_api_key_env(),
            api_key_file: None,
            agent_name: None,
            base_url: default_moltbook_base_url(),
            timeout_secs: default_moltbook_timeout_secs(),
            max_response_bytes: default_moltbook_max_response_bytes(),
        }
    }
}

/// Optional Google Workspace credentials (Gmail + Calendar APIs via domain-wide delegation). When `enabled`, both paths must be set; Admin Console must allow `https://mail.google.com/` and `https://www.googleapis.com/auth/calendar` for the service account client id.
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

/// Multimodal vision (`vision:see`, web drop zone). Master switch: when false, no mmproj spawn, no upload routes, no tool.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct VisionConfig {
    #[serde(default = "default_vision_enabled")]
    pub enabled: bool,
    /// Vault-relative directory for normalized uploads (e.g. `99_USER_UPLOADED/images`).
    #[serde(default = "default_vision_upload_dir")]
    pub upload_dir: String,
    /// Longest edge (px) after server-side normalize (Gemma encoder tile size).
    #[serde(default = "default_vision_target_max_px")]
    pub target_max_px: u32,
    /// Reject raw multipart uploads above this size (bytes).
    #[serde(default = "default_vision_max_upload_bytes")]
    pub max_upload_bytes: u64,
    /// Reject normalized JPEG output above this size (bytes).
    #[serde(default = "default_vision_max_output_bytes")]
    pub max_output_bytes: u64,
    #[serde(default = "default_vision_allowed_extensions")]
    pub allowed_extensions: Vec<String>,
    #[serde(default = "default_vision_jpeg_quality")]
    pub jpeg_quality: u8,
    #[serde(default = "default_vision_default_prompt")]
    pub default_prompt: String,
}

fn default_vision_enabled() -> bool {
    false
}

fn default_vision_upload_dir() -> String {
    "99_USER_UPLOADED/images".into()
}

fn default_vision_target_max_px() -> u32 {
    896
}

fn default_vision_max_upload_bytes() -> u64 {
    10 * 1024 * 1024
}

fn default_vision_max_output_bytes() -> u64 {
    2 * 1024 * 1024
}

fn default_vision_allowed_extensions() -> Vec<String> {
    vec![
        "png".into(),
        "jpg".into(),
        "jpeg".into(),
        "webp".into(),
        "gif".into(),
    ]
}

fn default_vision_jpeg_quality() -> u8 {
    85
}

fn default_vision_default_prompt() -> String {
    "Describe this image in detail for the user.".into()
}

impl Default for VisionConfig {
    fn default() -> Self {
        Self {
            enabled: default_vision_enabled(),
            upload_dir: default_vision_upload_dir(),
            target_max_px: default_vision_target_max_px(),
            max_upload_bytes: default_vision_max_upload_bytes(),
            max_output_bytes: default_vision_max_output_bytes(),
            allowed_extensions: default_vision_allowed_extensions(),
            jpeg_quality: default_vision_jpeg_quality(),
            default_prompt: default_vision_default_prompt(),
        }
    }
}

/// Voice ingress (STT before orchestrator turn). Master switch: when false, no upload routes, no transcription.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct AudioConfig {
    #[serde(default = "default_audio_enabled")]
    pub enabled: bool,
    /// Vault-relative directory for normalized uploads (e.g. `99_USER_UPLOADED/audio`).
    #[serde(default = "default_audio_upload_dir")]
    pub upload_dir: String,
    #[serde(default = "default_audio_max_upload_bytes")]
    pub max_upload_bytes: u64,
    /// Trim transcoded output to this many seconds (Gemma 4 practical limit).
    #[serde(default = "default_audio_max_duration_secs")]
    pub max_duration_secs: u32,
    #[serde(default = "default_audio_target_sample_rate")]
    pub target_sample_rate: u32,
    #[serde(default = "default_audio_target_channels")]
    pub target_channels: u16,
    #[serde(default = "default_audio_allowed_extensions")]
    pub allowed_extensions: Vec<String>,
    /// Fixed prompt for the llama-server STT call (verbatim transcription).
    #[serde(default = "default_audio_transcription_prompt")]
    pub transcription_prompt: String,
    /// Remove normalized WAV files under `upload_dir` when chat exits.
    #[serde(default = "default_audio_cleanup_uploads_on_chat_exit")]
    pub cleanup_uploads_on_chat_exit: bool,
}

fn default_audio_enabled() -> bool {
    false
}

fn default_audio_upload_dir() -> String {
    "99_USER_UPLOADED/audio".into()
}

fn default_audio_max_upload_bytes() -> u64 {
    10 * 1024 * 1024
}

fn default_audio_max_duration_secs() -> u32 {
    30
}

fn default_audio_target_sample_rate() -> u32 {
    16000
}

fn default_audio_target_channels() -> u16 {
    1
}

fn default_audio_allowed_extensions() -> Vec<String> {
    vec![
        "wav".into(),
        "mp3".into(),
        "m4a".into(),
        "ogg".into(),
        "webm".into(),
        "flac".into(),
    ]
}

fn default_audio_transcription_prompt() -> String {
    "Transcribe the speech verbatim. Output only the spoken words.".into()
}

fn default_audio_cleanup_uploads_on_chat_exit() -> bool {
    true
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            enabled: default_audio_enabled(),
            upload_dir: default_audio_upload_dir(),
            max_upload_bytes: default_audio_max_upload_bytes(),
            max_duration_secs: default_audio_max_duration_secs(),
            target_sample_rate: default_audio_target_sample_rate(),
            target_channels: default_audio_target_channels(),
            allowed_extensions: default_audio_allowed_extensions(),
            transcription_prompt: default_audio_transcription_prompt(),
            cleanup_uploads_on_chat_exit: default_audio_cleanup_uploads_on_chat_exit(),
        }
    }
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

fn default_web_fetch_chunk_num_ctx_ratio() -> f32 {
    0.9
}

fn default_web_fetch_user_agent() -> String {
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36"
        .to_string()
}

fn default_news_today_enabled() -> bool {
    true
}

fn default_news_today_max_headlines() -> usize {
    12
}

fn default_news_today_deep_fetch_max() -> u8 {
    0
}

fn default_news_today_site_base() -> String {
    "https://www.bbc.com".to_string()
}

fn default_news_today_default_homepage() -> Option<String> {
    Some("https://www.bbc.com/".to_string())
}

/// Anti-crawl and fetch budgets for `web:fetch`, `web:find`, and internal `news:today` fetches.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct WebConfig {
    #[serde(default = "default_web_default_fetch_budget")]
    pub default_fetch_budget: u32,
    #[serde(default = "default_web_max_fetch_budget_override")]
    pub max_fetch_budget_override: u32,
    #[serde(default = "default_web_max_fetches_per_user_turn")]
    pub max_fetches_per_user_turn: u32,
    #[serde(default = "default_web_max_fetches_per_chat_session")]
    pub max_fetches_per_chat_session: u32,
    #[serde(default = "default_web_max_fetches_per_mission")]
    pub max_fetches_per_mission: u32,
    #[serde(default = "default_web_max_web_tool_calls_per_turn")]
    pub max_web_tool_calls_per_turn: u32,
    #[serde(default = "default_web_require_find_before_refetch")]
    pub require_find_before_refetch: bool,
    #[serde(default)]
    pub explore_site_enabled: bool,
    /// When true, register `web:search` (browser39 `[search].engine` → allowlisted fetch).
    #[serde(default = "default_web_search_enabled")]
    pub search_enabled: bool,
    /// When true, load/save `{vault}/.fcp/web_session.json` across process restarts. Chat bootstrap still resets by default.
    #[serde(default)]
    pub persist_ledger: bool,
    /// When false, skip `.fcp/web_allowlist.toml` checks (useful for local dev; default on for safety).
    #[serde(default = "default_web_allowlist_enabled")]
    pub allowlist_enabled: bool,
    /// Try browser39 link-by-text clicks when fetched markdown is thin (see `consent_profiles.toml`).
    #[serde(default = "default_web_consent_helper_enabled")]
    pub consent_helper_enabled: bool,
    /// Persist browser39 cookies under `.fcp/browser39/sessions/hosts/{host}/` (required for multi-step consent).
    #[serde(default)]
    pub persist_browser39_sessions: bool,
    #[serde(default = "default_web_thin_page_char_threshold")]
    pub thin_page_char_threshold: usize,
    #[serde(default = "default_web_consent_max_attempts")]
    pub consent_max_attempts: u32,
    /// When true, never use host sessions / consent batch (one-shot `batch --no-persist` per artifact only).
    #[serde(default)]
    pub use_legacy_batch: bool,
    /// Remove `20_Discourse/web/missions/*` when a chat session ends (`/exit`, web shutdown, SIGINT).
    #[serde(default = "default_web_cleanup_missions_on_chat_exit")]
    pub cleanup_missions_on_chat_exit: bool,
    /// When true, chat startup runs `browser39 --version` and aborts if missing (see `docs/WEB_BROWSER39.md`).
    #[serde(default = "default_web_require_browser39")]
    pub require_browser39: bool,
}

fn default_web_require_browser39() -> bool {
    true
}

fn default_web_cleanup_missions_on_chat_exit() -> bool {
    true
}

fn default_web_allowlist_enabled() -> bool {
    true
}

fn default_web_search_enabled() -> bool {
    true
}

fn default_web_default_fetch_budget() -> u32 {
    2
}

fn default_web_max_fetch_budget_override() -> u32 {
    5
}

fn default_web_max_fetches_per_user_turn() -> u32 {
    2
}

fn default_web_max_fetches_per_chat_session() -> u32 {
    12
}

fn default_web_max_fetches_per_mission() -> u32 {
    4
}

fn default_web_max_web_tool_calls_per_turn() -> u32 {
    2
}

fn default_web_require_find_before_refetch() -> bool {
    true
}

fn default_web_consent_helper_enabled() -> bool {
    true
}

fn default_web_thin_page_char_threshold() -> usize {
    300
}

fn default_web_consent_max_attempts() -> u32 {
    2
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            default_fetch_budget: default_web_default_fetch_budget(),
            max_fetch_budget_override: default_web_max_fetch_budget_override(),
            max_fetches_per_user_turn: default_web_max_fetches_per_user_turn(),
            max_fetches_per_chat_session: default_web_max_fetches_per_chat_session(),
            max_fetches_per_mission: default_web_max_fetches_per_mission(),
            max_web_tool_calls_per_turn: default_web_max_web_tool_calls_per_turn(),
            require_find_before_refetch: default_web_require_find_before_refetch(),
            explore_site_enabled: false,
            search_enabled: default_web_search_enabled(),
            persist_ledger: false,
            allowlist_enabled: default_web_allowlist_enabled(),
            consent_helper_enabled: default_web_consent_helper_enabled(),
            persist_browser39_sessions: false,
            thin_page_char_threshold: default_web_thin_page_char_threshold(),
            consent_max_attempts: default_web_consent_max_attempts(),
            use_legacy_batch: false,
            cleanup_missions_on_chat_exit: default_web_cleanup_missions_on_chat_exit(),
            require_browser39: default_web_require_browser39(),
        }
    }
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

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
pub enum LlmBackend {
    #[default]
    Ollama,
    LlamaCpp,
}

impl std::fmt::Display for LlmBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ollama => write!(f, "Ollama"),
            Self::LlamaCpp => write!(f, "llama.cpp"),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct LlamaCppConfig {
    /// Path to the llama.cpp build directory (contains bin/llama-server).
    pub home: PathBuf,
    /// Host:port for the chat model llama-server instance.
    #[serde(default = "default_llamacpp_chat_server_url")]
    pub chat_server_url: String,
    /// Host:port for the embedding model llama-server instance.
    #[serde(default = "default_llamacpp_embed_server_url")]
    pub embed_server_url: String,
    /// GGUF model file for chat.
    pub chat_model_path: PathBuf,
    /// GGUF model file for embeddings.
    pub embed_model_path: PathBuf,
    /// GPU layers to offload (--n-gpu-layers); 0 = CPU only.
    #[serde(default)]
    pub n_gpu_layers: u32,
    /// Max seconds to wait for each llama-server to become ready after spawn.
    #[serde(default = "default_llamacpp_ready_timeout")]
    pub ready_timeout_secs: u64,
    /// When true, Eris leaves managed llama-server processes running on chat exit (detach/orphan).
    /// Use when abrupt GPU teardown drops the desktop session; stop servers manually later.
    #[serde(default)]
    pub detach_servers_on_chat_exit: bool,
    /// Seconds to wait after SIGTERM before optional SIGKILL when stopping managed llama-servers.
    #[serde(default = "default_llamacpp_shutdown_grace_secs")]
    pub shutdown_grace_secs: u64,
    /// Pause between stopping embed and chat llama-server (VRAM release stagger).
    #[serde(default = "default_llamacpp_shutdown_stagger_secs")]
    pub shutdown_stagger_secs: u64,
    /// When false (default), Eris never SIGKILLs managed llama-servers — only SIGTERM then detach.
    #[serde(default)]
    pub shutdown_allow_sigkill: bool,
    /// Multimodal projector GGUF; required when [`crate::config::VisionConfig::enabled`] is true.
    #[serde(default)]
    pub mmproj_path: Option<PathBuf>,
    /// Root for llama-server `--media-path` (`file://` relative URLs). Defaults to active vault at spawn.
    #[serde(default)]
    pub media_path: Option<PathBuf>,
}

pub(crate) fn default_llamacpp_ready_timeout() -> u64 {
    30
}

fn default_llamacpp_shutdown_grace_secs() -> u64 {
    30
}

fn default_llamacpp_shutdown_stagger_secs() -> u64 {
    3
}

impl Default for LlamaCppConfig {
    fn default() -> Self {
        Self {
            home: PathBuf::new(),
            chat_server_url: default_llamacpp_chat_server_url(),
            embed_server_url: default_llamacpp_embed_server_url(),
            chat_model_path: PathBuf::new(),
            embed_model_path: PathBuf::new(),
            n_gpu_layers: 0,
            ready_timeout_secs: default_llamacpp_ready_timeout(),
            detach_servers_on_chat_exit: false,
            shutdown_grace_secs: default_llamacpp_shutdown_grace_secs(),
            shutdown_stagger_secs: default_llamacpp_shutdown_stagger_secs(),
            shutdown_allow_sigkill: false,
            mmproj_path: None,
            media_path: None,
        }
    }
}

fn default_llamacpp_chat_server_url() -> String {
    "http://127.0.0.1:8090".into()
}

fn default_llamacpp_embed_server_url() -> String {
    "http://127.0.0.1:8091".into()
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
    /// Active LLM backend; existing vaults without this key default to Ollama.
    #[serde(default)]
    pub llm_backend: LlmBackend,
    /// Operator display name for UI / prompts; from TOML or `FCP_USER_NAME`; empty if unset.
    #[serde(default)]
    pub user_name: String,
    /// Context window: Ollama `num_ctx`, orchestrator budgets / condensation, and managed
    /// `llama-server --ctx-size` for the **chat** server when [`LlmBackend::LlamaCpp`] is active.
    /// The managed **embedding** server uses `min(num_ctx, 8192)` for `--ctx-size` so the second
    /// GPU process does not reserve full chat KV (see `executive::peripherals`).
    pub num_ctx: usize,
    /// Optional Ollama GPU layer count override (`num_gpu`). `None` uses Ollama auto-placement.
    pub ollama_num_gpu: Option<u32>,
    /// Optional Ollama primary GPU index (`main_gpu`) for multi-GPU systems.
    pub ollama_main_gpu: Option<u32>,
    /// Optional Ollama low-VRAM mode toggle (`low_vram`). `None` leaves runtime default.
    pub ollama_low_vram: Option<bool>,
    /// llama.cpp-specific config; `None` when backend is Ollama.
    #[serde(default)]
    pub llama_cpp: Option<LlamaCppConfig>,
    /// Max seconds to wait for a single LLM generation (connect + stream).
    pub generation_timeout_secs: u64,
    /// Floor for `eris benchmark` per-scenario wall clock: effective timeout is
    /// `max(scenario.timeout_seconds from the suite, this value)`. Use this for slow or layer-offloaded
    /// models so harness `tokio::time::timeout` matches your LLM budget (often set similar to
    /// [`Self::generation_timeout_secs`]). Default **120**.
    #[serde(default = "default_benchmark_scenario_timeout_secs")]
    pub benchmark_scenario_timeout_secs: u64,
    /// Override `--ctx-size` for the managed **chat** `llama-server` when the benchmark runner
    /// spawns peripherals.  Benchmarks use much smaller prompts than long interactive sessions;
    /// reserving the full chat `num_ctx` (e.g. 64 K) allocates enormous KV cache VRAM that is
    /// never filled, and the resulting memory pressure can destabilise the display stack on
    /// dual-GPU (iGPU + dGPU) systems.  `0` (default) means "use `num_ctx` unchanged."
    #[serde(default)]
    pub benchmark_num_ctx: usize,
    /// Milliseconds to sleep between benchmark scenarios, giving the GPU power/thermal state
    /// time to settle.  Helps on systems where rapid back-to-back completions cause transient
    /// PCIe / display link instability.  Default **500** ms.
    #[serde(default = "default_benchmark_inter_scenario_cooldown_ms")]
    pub benchmark_inter_scenario_cooldown_ms: u64,
    /// Forwarded to Ollama on each chat request as `.think(...)` in `OllamaClient::generate` (`ollama-rs` `ChatMessageRequest`), and to llama-server as `chat_template_kwargs` (`{"enable_thinking": false}` when `false`). When Eris spawns the managed chat `llama-server`, `false` also adds `--reasoning off` and `--reasoning-budget 0` (embed server unchanged). If chat is already listening on the configured port, you must pass equivalent flags on that process yourself. `false` (default) turns off the separate thinking/reasoning channel for models that support it—saves tokens and RAM versus `true`. TOML key name is historical; unrelated to `engine::router::ReasoningRouter`.
    pub enable_reasoning_fsm: bool,
    /// Fraction of estimated context fill (0.0–1.0) at which rolling condensation runs.
    pub condensation_threshold: f32,
    /// When **≥ 4096**, caps the post-compaction estimated stack ceiling to at most this many
    /// tokens (same cheap proxy as [`crate::orchestrator::context::estimate_stack_tokens`]). Values
    /// below 4096 are ignored so legacy placeholder values (for example `300`) do not collapse the stack.
    pub condensation_target: usize,
    /// Fraction of `num_ctx` kept as verbatim tail when planning a fold (`0.05`–`0.95`). Replaces the
    /// former hardcoded **0.55** in [`crate::orchestrator::context::retain_budget_tokens`].
    #[serde(default = "default_condensation_retain_ratio")]
    pub condensation_retain_ratio: f32,
    /// Max LLM summarization passes per [`crate::orchestrator::core::Orchestrator::execute_condensation`]
    /// call while the stack is still above the estimated ceiling.
    #[serde(default = "default_condensation_max_chained_passes")]
    pub condensation_max_chained_passes: usize,
    /// Estimated stack ceiling as `floor(num_ctx * ratio)` (combined with [`Self::condensation_target`]
    /// when that target is large enough to matter).
    #[serde(default = "default_condensation_stack_est_ceiling_ratio")]
    pub condensation_stack_est_ceiling_ratio: f32,
    /// Strip JSON-repair / recovery system rows from the stack tail before folding.
    #[serde(default = "default_condensation_strip_recovery_system_messages")]
    pub condensation_strip_recovery_system_messages: bool,
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
    /// When false, `news:today` is not registered.
    #[serde(default = "default_news_today_enabled")]
    pub news_today_enabled: bool,
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
    /// Optional override for persisted mission vault chunk size (`20_Discourse/web/missions/.../chunks/`).
    /// When unset, uses [`Self::num_ctx`] × [`Self::web_fetch_chunk_num_ctx_ratio`]. Always capped by that product.
    /// Does not affect `web:find` snippet caps (those still use [`Self::vault_read_ratio`]).
    #[serde(default)]
    pub web_fetch_chunk_chars: Option<usize>,
    /// Fraction of [`Self::num_ctx`] used as the default and ceiling for [`Self::web_fetch_chunk_chars`] (default `0.9`).
    #[serde(default = "default_web_fetch_chunk_num_ctx_ratio")]
    pub web_fetch_chunk_num_ctx_ratio: f32,
    /// `User-Agent` for `web:fetch`; combined with browser-like `Accept` / `Sec-Fetch-*` headers on the HTTP client.
    #[serde(default = "default_web_fetch_user_agent")]
    pub web_fetch_user_agent: String,
    /// When set, used as the HTTP `Referer` for `web:fetch` if the tool call does not pass `referer` (e.g. `https://www.google.com/` or the site homepage — reduces some CDN/bot 403s; not guaranteed).
    #[serde(default)]
    pub web_fetch_default_referer: Option<String>,
    /// Origin used by `news:today` to resolve `category` into a listing URL (paths like `news/politics`, `sport`). Default `https://www.bbc.com`.
    #[serde(default = "default_news_today_site_base")]
    pub news_today_site_base: String,
    /// Default listing URL when `news:today` omits `homepage_url` and `category` (default BBC home `https://www.bbc.com/` — site-wide top stories; use `category` for section fronts).
    #[serde(default = "default_news_today_default_homepage")]
    pub news_today_default_homepage: Option<String>,
    /// Default cap on headline rows returned by `news:today` (hard ceiling applied in tool).
    #[serde(default = "default_news_today_max_headlines")]
    pub news_today_max_headlines_default: usize,
    /// Default number of top-ranked article URLs to fetch after the homepage (0 = headlines only).
    #[serde(default = "default_news_today_deep_fetch_max")]
    pub news_today_deep_fetch_max_default: u8,
    /// Fraction of [`Self::num_ctx`] used to cap vault read and `web:find` snippet budgets (not persisted web chunk size).
    pub vault_read_ratio: f32,
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
    /// When true, after a graceful chat exit, run `ollama stop` for the chat and embedding models so GPU/RAM drops even if Eris did **not** start `ollama serve` (for example Ollama.app is running). Ignored when this session spawned and already tore down its own Ollama child.
    #[serde(default = "default_unload_ollama_models_on_chat_exit")]
    pub unload_ollama_models_on_chat_exit: bool,
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
    /// Multimodal vision feature gate (tool, upload routes, mmproj spawn).
    #[serde(default)]
    pub vision: VisionConfig,
    /// Voice ingress (STT before orchestrator turn, web upload routes).
    #[serde(default)]
    pub audio: AudioConfig,
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
    /// Optional Discord bot sidecar (same session as web/TUI).
    #[serde(default)]
    pub discord: DiscordConfig,
    /// Optional Moltbook social-network tools.
    #[serde(default)]
    pub moltbook: MoltbookConfig,
    /// Web fetch anti-crawl ledger, budgets, and find-before-refetch policy.
    #[serde(default)]
    pub web: WebConfig,
    /// Optional override path for `.fcp/web_allowlist.toml`.
    #[serde(default)]
    pub web_allowlist_path: Option<std::path::PathBuf>,
    /// When true, keep full JSON parameter schemas in the LLM view for tool definitions (larger prompt). When false and [`Self::optimize_context`] is true, [`crate::orchestrator::context::build_llm_view`] strips `parameters` in that block only; [`crate::orchestrator::core::Orchestrator::chat_stack`] stays full. Independently, the orchestrator forces full schemas for one recovery LLM pass after a Gatekeeper schema fault ([`crate::orchestrator::core::Orchestrator::force_full_tool_schemas_in_llm_view`]).
    #[serde(default = "default_optimize_context_full_tool_schemas")]
    pub optimize_context_full_tool_schemas: bool,
    /// When true and [`Self::optimize_context`] is true, collapse resolved tool-recovery spans in the LLM view only (canonical [`crate::orchestrator::core::Orchestrator::chat_stack`] unchanged).
    #[serde(default = "default_optimize_context_omit_resolved_tool_recovery")]
    pub optimize_context_omit_resolved_tool_recovery: bool,
    /// When true and [`Self::optimize_context`] is true, replace non-protocol assistant rows in the LLM view with a short placeholder (canonical stack unchanged).
    #[serde(default = "default_optimize_context_assistant_non_json_placeholder")]
    pub optimize_context_assistant_non_json_placeholder: bool,
    /// When true, run sliding-window condensation once before the main LLM `generate` if estimated stack tokens exceed `num_ctx * condensation_threshold * optimize_context_proactive_condensation_ratio` (same token proxy as [`crate::orchestrator::context::estimate_stack_tokens`]).
    #[serde(default = "default_optimize_context_proactive_condensation")]
    pub optimize_context_proactive_condensation: bool,
    /// Scales the proactive condensation trigger (typical range `0.7`–`0.95`; lower = fold earlier). Ignored when proactive condensation is disabled.
    #[serde(default = "default_optimize_context_proactive_condensation_ratio")]
    pub optimize_context_proactive_condensation_ratio: f32,
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
    /// Max files returned by [`crate::tools::vault::VaultSearchTool`] (caps LLM `max_files`).
    #[serde(default = "default_vault_search_max_files")]
    pub vault_search_max_files: u32,
    /// Max excerpt lines per file in `vault:search` (distinct hit lines, before radius merge).
    #[serde(default = "default_vault_search_max_snippets_per_file")]
    pub vault_search_max_snippets_per_file: u32,
    /// Lines above/below each hit line to include in `vault:search` excerpts.
    #[serde(default = "default_vault_search_snippet_radius_lines")]
    pub vault_search_snippet_radius_lines: u32,
    /// Total character budget for formatted `vault:search` output.
    #[serde(default = "default_vault_search_max_total_chars")]
    pub vault_search_max_total_chars: usize,
    /// Skip reading files larger than this many bytes (lexical scan only).
    #[serde(default = "default_vault_search_max_file_bytes")]
    pub vault_search_max_file_bytes: u64,
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

fn default_unload_ollama_models_on_chat_exit() -> bool {
    true
}

fn default_benchmark_scenario_timeout_secs() -> u64 {
    120
}

fn default_benchmark_inter_scenario_cooldown_ms() -> u64 {
    500
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

fn default_optimize_context_assistant_non_json_placeholder() -> bool {
    true
}

fn default_optimize_context_proactive_condensation() -> bool {
    false
}

fn default_condensation_retain_ratio() -> f32 {
    0.55
}

fn default_condensation_max_chained_passes() -> usize {
    3
}

fn default_condensation_stack_est_ceiling_ratio() -> f32 {
    0.92
}

fn default_condensation_strip_recovery_system_messages() -> bool {
    true
}

fn default_optimize_context_proactive_condensation_ratio() -> f32 {
    0.85
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

fn default_vault_search_max_files() -> u32 {
    10
}

fn default_vault_search_max_snippets_per_file() -> u32 {
    3
}

fn default_vault_search_snippet_radius_lines() -> u32 {
    1
}

fn default_vault_search_max_total_chars() -> usize {
    12_000
}

fn default_vault_search_max_file_bytes() -> u64 {
    1_048_576
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

/// v6.db.transport.rest profiles for [`crate::tools::db_rest::DbFindConnectionsTool`] (`/locations`, `/journeys`).
pub fn default_db_transport_rest_apis() -> HashMap<String, ApiProfile> {
    let mut m = HashMap::new();
    m.insert(
        "db_rest_locations".into(),
        ApiProfile {
            enabled: true,
            base_url: "https://v6.db.transport.rest/locations".into(),
            query: [
                ("query".into(), "{query}".into()),
                ("results".into(), "1".into()),
            ]
            .into_iter()
            .collect(),
            headers: HashMap::new(),
            max_response_bytes: Some(65_536),
            stale_after_secs: None,
        },
    );
    m.insert(
        "db_rest_journeys_departure".into(),
        ApiProfile {
            enabled: true,
            base_url: "https://v6.db.transport.rest/journeys".into(),
            query: [
                ("from".into(), "{from}".into()),
                ("to".into(), "{to}".into()),
                ("departure".into(), "{when}".into()),
                ("results".into(), "3".into()),
            ]
            .into_iter()
            .collect(),
            headers: HashMap::new(),
            max_response_bytes: Some(786_432),
            stale_after_secs: None,
        },
    );
    m.insert(
        "db_rest_journeys_arrival".into(),
        ApiProfile {
            enabled: true,
            base_url: "https://v6.db.transport.rest/journeys".into(),
            query: [
                ("from".into(), "{from}".into()),
                ("to".into(), "{to}".into()),
                ("arrival".into(), "{when}".into()),
                ("results".into(), "3".into()),
            ]
            .into_iter()
            .collect(),
            headers: HashMap::new(),
            max_response_bytes: Some(786_432),
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

/// Weather + Wikipedia + DB timetable profiles merged into [`AppConfig::default`]; TOML `[apis]` entries override by id.
fn default_builtin_apis() -> HashMap<String, ApiProfile> {
    let mut apis = default_open_meteo_apis();
    apis.extend(default_wikipedia_page_summary_api());
    apis.extend(default_db_transport_rest_apis());
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
            llm_backend: LlmBackend::default(),
            user_name: String::new(),
            num_ctx: 16384,
            ollama_num_gpu: None,
            ollama_main_gpu: None,
            ollama_low_vram: None,
            llama_cpp: None,
            generation_timeout_secs: 120,
            benchmark_scenario_timeout_secs: default_benchmark_scenario_timeout_secs(),
            benchmark_num_ctx: 0,
            benchmark_inter_scenario_cooldown_ms: default_benchmark_inter_scenario_cooldown_ms(),
            enable_reasoning_fsm: false,
            condensation_threshold: 0.5,
            condensation_target: 0,
            condensation_retain_ratio: default_condensation_retain_ratio(),
            condensation_max_chained_passes: default_condensation_max_chained_passes(),
            condensation_stack_est_ceiling_ratio: default_condensation_stack_est_ceiling_ratio(),
            condensation_strip_recovery_system_messages:
                default_condensation_strip_recovery_system_messages(),
            max_tool_rounds: 5,
            max_recovery_attempts: 3,
            ephemeral_ttl_session_secs: default_ephemeral_ttl_session_secs(),
            ephemeral_ttl_scratch_secs: default_ephemeral_ttl_scratch_secs(),
            ephemeral_ttl_promote_secs: default_ephemeral_ttl_promote_secs(),
            promotion_threshold_session_to_scratch: default_promotion_threshold_session_to_scratch(
            ),
            promotion_threshold_scratch_to_promote: default_promotion_threshold_scratch_to_promote(
            ),
            promotion_decay_per_tick: default_promotion_decay_per_tick(),
            promotion_eval_interval_secs: default_promotion_eval_interval_secs(),
            promotion_mention_boost: default_promotion_mention_boost(),
            promotion_stage_boost: default_promotion_stage_boost(),
            turn_end_mention_enabled: default_turn_end_mention_enabled(),
            staged_memory_prompt_max_chars: default_staged_memory_prompt_max_chars(),
            news_today_enabled: default_news_today_enabled(),
            qdrant_url: "http://localhost:6334".into(),
            qdrant_collection_v2: "fcp_vault_v2_default".into(),
            snapshot_interval_secs: 300,
            embed_model_name: "nomic-embed-text".into(),
            idle_timeout_secs: 900,
            idle_heartbeat_enabled: false,
            web_fetch_timeout_secs: 10,
            web_fetch_max_bytes: 20480,
            web_fetch_chunk_chars: None,
            web_fetch_chunk_num_ctx_ratio: default_web_fetch_chunk_num_ctx_ratio(),
            web_fetch_user_agent: default_web_fetch_user_agent(),
            web_fetch_default_referer: None,
            news_today_site_base: default_news_today_site_base(),
            news_today_default_homepage: default_news_today_default_homepage(),
            news_today_max_headlines_default: default_news_today_max_headlines(),
            news_today_deep_fetch_max_default: default_news_today_deep_fetch_max(),
            vault_read_ratio: 0.5,
            tool_match_threshold: 0.50,
            tool_descriptor_jit_top_k: default_tool_descriptor_jit_top_k(),
            tool_descriptor_jit_max_chars: default_tool_descriptor_jit_max_chars(),
            slim_tool_prompt: default_slim_tool_prompt(),
            tool_map_offer_cap: default_tool_map_offer_cap(),
            ollama_daemon: default_ollama_daemon(),
            unload_ollama_models_on_chat_exit: default_unload_ollama_models_on_chat_exit(),
            qdrant_daemon: default_qdrant_daemon(),
            require_semantic_brain: default_require_semantic_brain(),
            semantic_brain_connect_attempts: default_semantic_brain_connect_attempts(),
            semantic_brain_connect_retry_delay_ms: default_semantic_brain_connect_retry_delay_ms(),
            apis: default_builtin_apis(),
            vault_watch: VaultWatchConfig::default(),
            vision: VisionConfig::default(),
            audio: AudioConfig::default(),
            optimize_context: default_optimize_context(),
            optimize_context_max_tool_snippet_chars:
                default_optimize_context_max_tool_snippet_chars(),
            optimize_context_assistant_compact: default_optimize_context_assistant_compact(),
            optimize_context_tool_overrides: HashMap::new(),
            google: GoogleConfig::default(),
            discord: DiscordConfig::default(),
            moltbook: MoltbookConfig::default(),
            web: WebConfig::default(),
            web_allowlist_path: None,

            optimize_context_full_tool_schemas: default_optimize_context_full_tool_schemas(),
            optimize_context_omit_resolved_tool_recovery:
                default_optimize_context_omit_resolved_tool_recovery(),
            optimize_context_assistant_non_json_placeholder:
                default_optimize_context_assistant_non_json_placeholder(),
            optimize_context_proactive_condensation:
                default_optimize_context_proactive_condensation(),
            optimize_context_proactive_condensation_ratio:
                default_optimize_context_proactive_condensation_ratio(),
            memory_query_default_top_k: default_memory_query_default_top_k(),
            memory_query_top_k_max: default_memory_query_top_k_max(),
            memory_query_default_max_total_chars: default_memory_query_default_max_total_chars(),
            memory_query_min_max_total_chars: default_memory_query_min_max_total_chars(),
            memory_query_oversample_cap: default_memory_query_oversample_cap(),
            memory_query_oversample_multiplier: default_memory_query_oversample_multiplier(),
            memory_query_oversample_min: default_memory_query_oversample_min(),
            vault_search_max_files: default_vault_search_max_files(),
            vault_search_max_snippets_per_file: default_vault_search_max_snippets_per_file(),
            vault_search_snippet_radius_lines: default_vault_search_snippet_radius_lines(),
            vault_search_max_total_chars: default_vault_search_max_total_chars(),
            vault_search_max_file_bytes: default_vault_search_max_file_bytes(),
            web_bind_addr: default_web_bind_addr(),
            web_port: default_web_port(),
            web_open_browser: default_web_open_browser(),
            config_source_dir: PathBuf::new(),
        }
    }
}

impl AppConfig {
    /// Persisted `web:fetch` mission chunk size in characters (UTF-8), for vault `chunks/NNN.md` files.
    ///
    /// Ceiling is [`Self::num_ctx`] × [`Self::web_fetch_chunk_num_ctx_ratio`] (default 90%). An explicit
    /// [`Self::web_fetch_chunk_chars`] is clamped to that ceiling. Minimum 512.
    pub fn resolved_web_fetch_chunk_chars(&self) -> usize {
        let ratio = self.web_fetch_chunk_num_ctx_ratio.clamp(0.1_f32, 1.0_f32);
        let cap = ((self.num_ctx.max(1) as f64) * f64::from(ratio)).floor() as usize;
        let cap = cap.max(512);
        let requested = self.web_fetch_chunk_chars.unwrap_or(cap);
        requested.min(cap).max(512)
    }

    /// Cheap-token ceiling for the full chat stack after condensation / hard trim.
    pub fn condensation_stack_est_ceiling_tokens(&self, num_ctx: usize) -> usize {
        let n = num_ctx.max(1);
        let ratio = self
            .condensation_stack_est_ceiling_ratio
            .clamp(0.55_f32, 1.0_f32);
        let derived = ((n as f32) * ratio).floor() as usize;
        let t = self.condensation_target;
        if t >= 4096 {
            derived.min(t)
        } else {
            derived
        }
    }

    pub fn load(cli: crate::executive::cli::Cli) -> crate::executive::error::Result<Self> {
        use figment::{
            Figment,
            providers::{Env, Format, Toml},
        };

        let _ = dotenvy::dotenv();

        let figment = Figment::from(figment::providers::Serialized::defaults(
            AppConfig::default(),
        ))
        .merge(Toml::file(crate::vault_layout::config_toml(
            std::path::Path::new("."),
        )))
        .merge(Env::prefixed("FCP_"));

        let mut config: AppConfig = figment
            .extract()
            .map_err(|e| crate::executive::error::FcpError::Config(e.to_string()))?;

        config.config_source_dir = std::env::current_dir().map_err(|e| {
            crate::executive::error::FcpError::Config(format!(
                "Could not read current directory: {}",
                e
            ))
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

    /// Bot token from [`Self::discord`] for the Serenity gateway (trimmed). Errors if missing or blank — see [`Self::discord_sidecar_should_run`].
    pub fn resolved_discord_bot_token(&self) -> crate::executive::error::Result<String> {
        let Some(ref t) = self.discord.bot_token else {
            return Err(crate::executive::error::FcpError::Config(
                "Set discord.bot_token in .fcp/config.toml (non-empty string) to run the Discord sidecar".into(),
            ));
        };
        let trimmed = t.trim();
        if trimmed.is_empty() {
            return Err(crate::executive::error::FcpError::Config(
                "discord.bot_token is empty in .fcp/config.toml".into(),
            ));
        }
        Ok(trimmed.to_string())
    }

    /// Whether the Serenity Discord sidecar should start. Requires [`DiscordConfig::enabled`], `application_id`,
    /// a listen channel, **and** a non-empty [`DiscordConfig::bot_token`] (gateway bots cannot use the public key alone).
    pub fn discord_sidecar_should_run(&self) -> bool {
        if !self.discord.enabled {
            return false;
        }
        if self.discord.application_id.is_none() {
            return false;
        }
        let has_channel = self.discord.channel_id.is_some()
            || self
                .discord
                .channel_name
                .as_ref()
                .is_some_and(|s| !s.trim().is_empty());
        if !has_channel {
            return false;
        }
        self.resolved_discord_bot_token().is_ok()
    }

    /// Validates Discord **metadata** when the feature flag is on: `application_id` and a channel target.
    /// A missing `bot_token` does **not** fail chat — the sidecar is simply skipped until you add one.
    pub fn validate_discord_sidecar(&self) -> crate::executive::error::Result<()> {
        if !self.discord.enabled {
            return Ok(());
        }
        if self.discord.application_id.is_none() {
            return Err(crate::executive::error::FcpError::Config(
                "Discord enabled: set discord.application_id (Developer Portal Application ID)"
                    .into(),
            ));
        }
        let has_channel = self.discord.channel_id.is_some()
            || self
                .discord
                .channel_name
                .as_ref()
                .is_some_and(|s| !s.trim().is_empty());
        if !has_channel {
            return Err(crate::executive::error::FcpError::Config(
                "Discord enabled: set discord.channel_id and/or discord.channel_name".into(),
            ));
        }
        Ok(())
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
    pub fn promotion_threshold_for_tier(
        &self,
        tier: crate::memory::types::EphemeralTier,
    ) -> Option<f64> {
        match tier {
            crate::memory::types::EphemeralTier::Session => {
                Some(self.promotion_threshold_session_to_scratch)
            }
            crate::memory::types::EphemeralTier::Scratch => {
                Some(self.promotion_threshold_scratch_to_promote)
            }
            crate::memory::types::EphemeralTier::Promote => None,
        }
    }

    pub fn is_llamacpp(&self) -> bool {
        self.llm_backend == LlmBackend::LlamaCpp
    }

    pub fn is_ollama(&self) -> bool {
        self.llm_backend == LlmBackend::Ollama
    }

    /// Validate llama.cpp config when backend is LlamaCpp.
    /// Returns Err if required paths are missing or llama-server binary not found.
    pub fn validate_llamacpp_config(&self) -> crate::executive::error::Result<&LlamaCppConfig> {
        if self.llm_backend != LlmBackend::LlamaCpp {
            return Err(crate::executive::error::FcpError::Config(
                "validate_llamacpp_config called but backend is not LlamaCpp".into(),
            ));
        }
        let lc = self.llama_cpp.as_ref().ok_or_else(|| {
            crate::executive::error::FcpError::Config(
                "[llama_cpp] section required when llm_backend = LlamaCpp".into(),
            )
        })?;
        let server_bin = lc.home.join("bin").join("llama-server");
        if !server_bin.exists() {
            return Err(crate::executive::error::FcpError::Config(format!(
                "llama-server binary not found at {}",
                server_bin.display()
            )));
        }
        if !lc.chat_model_path.exists() {
            return Err(crate::executive::error::FcpError::Config(format!(
                "Chat GGUF not found: {}",
                lc.chat_model_path.display()
            )));
        }
        if !lc.embed_model_path.exists() {
            return Err(crate::executive::error::FcpError::Config(format!(
                "Embed GGUF not found: {}",
                lc.embed_model_path.display()
            )));
        }
        Ok(lc)
    }

    /// When [`VisionConfig::enabled`], require LlamaCpp backend and a present `mmproj_path`.
    pub fn validate_vision_ready(&self) -> crate::executive::error::Result<()> {
        if !self.vision.enabled {
            return Ok(());
        }
        if !self.is_llamacpp() {
            return Err(crate::executive::error::FcpError::Config(
                "[vision] enabled requires llm_backend = LlamaCpp".into(),
            ));
        }
        let lc = self.llama_cpp.as_ref().ok_or_else(|| {
            crate::executive::error::FcpError::Config(
                "[vision] enabled requires [llama_cpp] section".into(),
            )
        })?;
        let mmproj = lc.mmproj_path.as_ref().ok_or_else(|| {
            crate::executive::error::FcpError::Config(
                "[vision] enabled requires llama_cpp.mmproj_path".into(),
            )
        })?;
        if !mmproj.exists() {
            return Err(crate::executive::error::FcpError::Config(format!(
                "[vision] mmproj not found: {}",
                mmproj.display()
            )));
        }
        Ok(())
    }

    /// When [`AudioConfig::enabled`], require LlamaCpp backend and a present `mmproj_path`.
    pub fn validate_audio_ready(&self) -> crate::executive::error::Result<()> {
        if !self.audio.enabled {
            return Ok(());
        }
        if !self.is_llamacpp() {
            return Err(crate::executive::error::FcpError::Config(
                "[audio] enabled requires llm_backend = LlamaCpp".into(),
            ));
        }
        let lc = self.llama_cpp.as_ref().ok_or_else(|| {
            crate::executive::error::FcpError::Config(
                "[audio] enabled requires [llama_cpp] section".into(),
            )
        })?;
        let mmproj = lc.mmproj_path.as_ref().ok_or_else(|| {
            crate::executive::error::FcpError::Config(
                "[audio] enabled requires llama_cpp.mmproj_path".into(),
            )
        })?;
        if !mmproj.exists() {
            return Err(crate::executive::error::FcpError::Config(format!(
                "[audio] mmproj not found: {}",
                mmproj.display()
            )));
        }
        Ok(())
    }

    /// True when either multimodal ingress feature needs mmproj on the chat llama-server.
    pub fn multimodal_mmproj_required(&self) -> bool {
        self.vision.enabled || self.audio.enabled
    }

    /// Physical directory for chat, ignition, tools, and `.fcp/` — always the process working directory
    /// at [`AppConfig::load`] (i.e. `cd` into your vault, then run `eris chat`).
    ///
    /// [`Self::workspace`] and [`Self::vault_root`] do **not** form this path; they remain logical / legacy config only.
    pub fn active_vault(&self) -> PathBuf {
        self.config_source_dir.clone()
    }

    /// Absolute paths under `chat_workspace_root` for vault watch (e.g. `00_Invariants/Identity.md`).
    pub fn resolved_vault_watch_file_paths(
        &self,
        chat_workspace_root: &std::path::Path,
    ) -> Vec<std::path::PathBuf> {
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
            jail.create_file(
                ".fcp/config.toml",
                r#"
                workspace = "toml_workspace"
                vault_root = "/toml/vaults"
                log_level = "warn"
            "#,
            )?;

            jail.create_file(
                ".env",
                r#"
                FCP_WORKSPACE=env_workspace
                FCP_LOG_LEVEL=error
            "#,
            )?;

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

        let parsed_config: AppConfig =
            serde_json::from_str(json_data).expect("Failed to parse JSON");

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
        assert_eq!(
            parsed_config.web_fetch_user_agent,
            default_web_fetch_user_agent()
        );
        assert_eq!(parsed_config.vault_read_ratio, 0.25);
        assert_eq!(
            parsed_config.resolved_web_fetch_chunk_chars(),
            (32_768_f32 * 0.9_f32).floor() as usize
        );
        assert_eq!(parsed_config.tool_match_threshold, 0.50);
        assert_eq!(parsed_config.ollama_daemon.command, "ollama");
        assert_eq!(parsed_config.ollama_daemon.args, vec!["serve"]);
        assert_eq!(parsed_config.unload_ollama_models_on_chat_exit, true);
        assert_eq!(
            parsed_config.benchmark_scenario_timeout_secs,
            default_benchmark_scenario_timeout_secs()
        );
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
        assert_eq!(c.apis.len(), 8);
        assert!(c.apis.contains_key("open_meteo_geocode"));
        assert!(c.apis.contains_key("open_meteo_geocode_cc"));
        assert!(c.apis.contains_key("open_meteo_forecast_current"));
        assert!(c.apis.contains_key("open_meteo_forecast_hourly"));
        assert!(c.apis.contains_key("wikipedia_page_summary"));
        assert!(c.apis.contains_key("db_rest_locations"));
        assert!(c.apis.contains_key("db_rest_journeys_departure"));
        assert!(c.apis.contains_key("db_rest_journeys_arrival"));
        let wiki = c.apis.get("wikipedia_page_summary").expect("wiki profile");
        assert!(wiki.headers.contains_key("User-Agent"));
    }

    #[test]
    fn default_config_has_v2_collection_name() {
        let c = AppConfig::default();
        assert_eq!(c.qdrant_collection_v2, "fcp_vault_v2_default");
    }

    #[test]
    fn default_benchmark_scenario_timeout_is_120() {
        let c = AppConfig::default();
        assert_eq!(c.benchmark_scenario_timeout_secs, 120);
        assert_eq!(
            c.benchmark_scenario_timeout_secs,
            default_benchmark_scenario_timeout_secs()
        );
    }

    #[test]
    fn ttl_for_tier_returns_configured_values() {
        let c = AppConfig::default();
        assert_eq!(
            c.ttl_for_tier(crate::memory::types::EphemeralTier::Session),
            900
        );
        assert_eq!(
            c.ttl_for_tier(crate::memory::types::EphemeralTier::Scratch),
            3600
        );
        assert_eq!(
            c.ttl_for_tier(crate::memory::types::EphemeralTier::Promote),
            28800
        );
    }

    #[test]
    fn promotion_threshold_returns_none_for_promote() {
        let c = AppConfig::default();
        assert!(
            c.promotion_threshold_for_tier(crate::memory::types::EphemeralTier::Session)
                .is_some()
        );
        assert!(
            c.promotion_threshold_for_tier(crate::memory::types::EphemeralTier::Scratch)
                .is_some()
        );
        assert!(
            c.promotion_threshold_for_tier(crate::memory::types::EphemeralTier::Promote)
                .is_none()
        );
    }

    #[test]
    fn vault_watch_includes_invariants_identity() {
        let c = AppConfig::default();
        assert!(
            c.vault_watch
                .paths
                .iter()
                .any(|p| p.contains("00_Invariants"))
        );
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

    #[test]
    fn round_trip_llamacpp_config() {
        let mut config = AppConfig::default();
        config.llm_backend = LlmBackend::LlamaCpp;
        config.num_ctx = 32768;
        config.llama_cpp = Some(LlamaCppConfig {
            home: PathBuf::from("/opt/llama.cpp/build"),
            chat_model_path: PathBuf::from("/models/chat.gguf"),
            embed_model_path: PathBuf::from("/models/embed.gguf"),
            n_gpu_layers: 99,
            ..Default::default()
        });

        let toml_str = toml::to_string(&config).expect("serialize");
        let deserialized: AppConfig = toml::from_str(&toml_str).expect("deserialize");

        assert_eq!(deserialized.llm_backend, LlmBackend::LlamaCpp);
        let lc = deserialized.llama_cpp.expect("llama_cpp section");
        assert_eq!(lc.home, PathBuf::from("/opt/llama.cpp/build"));
        assert_eq!(lc.chat_server_url, "http://127.0.0.1:8090");
        assert_eq!(lc.embed_server_url, "http://127.0.0.1:8091");
        assert_eq!(lc.chat_model_path, PathBuf::from("/models/chat.gguf"));
        assert_eq!(lc.embed_model_path, PathBuf::from("/models/embed.gguf"));
        assert_eq!(deserialized.num_ctx, 32768);
        assert_eq!(lc.n_gpu_layers, 99);
    }

    #[test]
    fn missing_backend_defaults_to_ollama() {
        let toml_str = r#"
            workspace = "test"
            vault_root = "/tmp"
            log_level = "info"
            ollama_host = "http://localhost:11434"
            model_name = "test:7b"
            num_ctx = 8192
            generation_timeout_secs = 60
            enable_reasoning_fsm = false
            condensation_threshold = 0.5
            condensation_target = 300
            max_tool_rounds = 5
            max_recovery_attempts = 3
            qdrant_url = "http://localhost:6334"
            snapshot_interval_secs = 300
            embed_model_name = "nomic-embed-text"
            idle_timeout_secs = 900
            web_fetch_timeout_secs = 10
            web_fetch_max_bytes = 20480
            vault_read_ratio = 0.5
            tool_match_threshold = 0.5
            [ollama_daemon]
            command = "ollama"
            args = ["serve"]
            [qdrant_daemon]
            command = "qdrant"
            args = []
        "#;
        let config: AppConfig = toml::from_str(toml_str).expect("deserialize");
        assert_eq!(config.llm_backend, LlmBackend::Ollama);
    }

    #[test]
    fn missing_llamacpp_section_is_none() {
        let toml_str = r#"
            workspace = "test"
            vault_root = "/tmp"
            log_level = "info"
            ollama_host = "http://localhost:11434"
            model_name = "test:7b"
            llm_backend = "Ollama"
            num_ctx = 8192
            generation_timeout_secs = 60
            enable_reasoning_fsm = false
            condensation_threshold = 0.5
            condensation_target = 300
            max_tool_rounds = 5
            max_recovery_attempts = 3
            qdrant_url = "http://localhost:6334"
            snapshot_interval_secs = 300
            embed_model_name = "nomic-embed-text"
            idle_timeout_secs = 900
            web_fetch_timeout_secs = 10
            web_fetch_max_bytes = 20480
            vault_read_ratio = 0.5
            tool_match_threshold = 0.5
            [ollama_daemon]
            command = "ollama"
            args = ["serve"]
            [qdrant_daemon]
            command = "qdrant"
            args = []
        "#;
        let config: AppConfig = toml::from_str(toml_str).expect("deserialize");
        assert_eq!(config.llm_backend, LlmBackend::Ollama);
        assert!(config.llama_cpp.is_none());
    }

    #[test]
    fn vision_defaults_disabled() {
        let config = AppConfig::default();
        assert!(!config.vision.enabled);
        assert_eq!(config.vision.upload_dir, "99_USER_UPLOADED/images");
        assert_eq!(config.vision.target_max_px, 896);
    }

    #[test]
    fn audio_defaults_disabled() {
        let config = AppConfig::default();
        assert!(!config.audio.enabled);
        assert_eq!(config.audio.upload_dir, "99_USER_UPLOADED/audio");
        assert_eq!(config.audio.max_duration_secs, 30);
        assert_eq!(config.audio.target_sample_rate, 16000);
    }

    #[test]
    fn validate_audio_ready_noop_when_disabled() {
        let config = AppConfig::default();
        assert!(config.validate_audio_ready().is_ok());
    }

    #[test]
    fn multimodal_mmproj_required_either_feature() {
        let mut config = AppConfig::default();
        assert!(!config.multimodal_mmproj_required());
        config.audio.enabled = true;
        assert!(config.multimodal_mmproj_required());
        config.audio.enabled = false;
        config.vision.enabled = true;
        assert!(config.multimodal_mmproj_required());
    }

    #[test]
    fn validate_vision_ready_noop_when_disabled() {
        let config = AppConfig::default();
        assert!(config.validate_vision_ready().is_ok());
    }

    #[test]
    fn validate_llamacpp_catches_missing_section() {
        let mut config = AppConfig::default();
        config.llm_backend = LlmBackend::LlamaCpp;
        config.llama_cpp = None;

        let err = config.validate_llamacpp_config().unwrap_err();
        assert!(err.to_string().contains("[llama_cpp] section required"));
    }

    #[test]
    fn validate_llamacpp_catches_missing_binary() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("bin")).expect("mkdir");

        let mut config = AppConfig::default();
        config.llm_backend = LlmBackend::LlamaCpp;
        config.llama_cpp = Some(LlamaCppConfig {
            home: tmp.path().to_path_buf(),
            chat_model_path: PathBuf::from("/fake/chat.gguf"),
            embed_model_path: PathBuf::from("/fake/embed.gguf"),
            ..Default::default()
        });

        let err = config.validate_llamacpp_config().unwrap_err();
        assert!(err.to_string().contains("llama-server binary not found"));
    }

    #[test]
    fn validate_llamacpp_catches_missing_gguf() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let bin_dir = tmp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("mkdir");
        std::fs::write(bin_dir.join("llama-server"), b"fake").expect("write");

        let mut config = AppConfig::default();
        config.llm_backend = LlmBackend::LlamaCpp;
        config.llama_cpp = Some(LlamaCppConfig {
            home: tmp.path().to_path_buf(),
            chat_model_path: PathBuf::from("/nonexistent/chat.gguf"),
            embed_model_path: PathBuf::from("/nonexistent/embed.gguf"),
            ..Default::default()
        });

        let err = config.validate_llamacpp_config().unwrap_err();
        assert!(err.to_string().contains("Chat GGUF not found"));
    }

    #[test]
    fn web_config_defaults_and_toml_table() {
        let defaults = WebConfig::default();
        assert_eq!(defaults.default_fetch_budget, 2);
        assert_eq!(defaults.max_fetch_budget_override, 5);
        assert_eq!(defaults.max_fetches_per_user_turn, 2);
        assert_eq!(defaults.max_fetches_per_chat_session, 12);
        assert_eq!(defaults.max_fetches_per_mission, 4);
        assert_eq!(defaults.max_web_tool_calls_per_turn, 2);
        assert!(defaults.require_find_before_refetch);
        assert!(!defaults.explore_site_enabled);
        assert!(!defaults.persist_ledger);
        assert!(defaults.consent_helper_enabled);
        assert_eq!(defaults.thin_page_char_threshold, 300);
        assert_eq!(defaults.consent_max_attempts, 2);
        assert!(defaults.cleanup_missions_on_chat_exit);
        assert!(defaults.require_browser39);

        let parsed: WebConfig = toml::from_str(
            r#"
            default_fetch_budget = 3
            max_fetches_per_chat_session = 20
            persist_ledger = true
            require_find_before_refetch = false
            "#,
        )
        .expect("web table");
        assert_eq!(parsed.default_fetch_budget, 3);
        assert_eq!(parsed.max_fetches_per_chat_session, 20);
        assert!(parsed.persist_ledger);
        assert!(!parsed.require_find_before_refetch);
    }

    #[test]
    fn is_llamacpp_and_is_ollama_helpers() {
        let mut config = AppConfig::default();
        assert!(config.is_ollama());
        assert!(!config.is_llamacpp());

        config.llm_backend = LlmBackend::LlamaCpp;
        assert!(config.is_llamacpp());
        assert!(!config.is_ollama());
    }

    #[test]
    fn default_urls_populated() {
        let toml_str = r#"
            workspace = "test"
            vault_root = "/tmp"
            log_level = "info"
            ollama_host = "http://localhost:11434"
            model_name = "test:7b"
            llm_backend = "LlamaCpp"
            num_ctx = 8192
            generation_timeout_secs = 60
            enable_reasoning_fsm = false
            condensation_threshold = 0.5
            condensation_target = 300
            max_tool_rounds = 5
            max_recovery_attempts = 3
            qdrant_url = "http://localhost:6334"
            snapshot_interval_secs = 300
            embed_model_name = "nomic-embed-text"
            idle_timeout_secs = 900
            web_fetch_timeout_secs = 10
            web_fetch_max_bytes = 20480
            vault_read_ratio = 0.5
            tool_match_threshold = 0.5
            [ollama_daemon]
            command = "ollama"
            args = ["serve"]
            [qdrant_daemon]
            command = "qdrant"
            args = []
            [llama_cpp]
            home = "/opt/llama.cpp/build"
            chat_model_path = "/models/chat.gguf"
            embed_model_path = "/models/embed.gguf"
        "#;
        let config: AppConfig = toml::from_str(toml_str).expect("deserialize");
        let lc = config.llama_cpp.expect("llama_cpp present");
        assert_eq!(lc.chat_server_url, "http://127.0.0.1:8090");
        assert_eq!(lc.embed_server_url, "http://127.0.0.1:8091");
        assert_eq!(config.num_ctx, 8192);
        assert_eq!(lc.n_gpu_layers, 0);
    }
}
