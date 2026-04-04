use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::tools::ToolContextViewHint;

/// HTTP API profile for [`crate::util::ApiHttpClient`] (URL/query/header templates). Map keys are profile ids (`[apis.<id>]` in `.fcp/config.toml`).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ApiProfile {
    #[serde(default = "default_api_profile_enabled")]
    pub enabled: bool,
    pub base_url: String,
    #[serde(default)]
    pub query: HashMap<String, String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// When unset, [`AppConfig::web_fetch_max_bytes`] is used.
    #[serde(default)]
    pub max_response_bytes: Option<usize>,
    #[serde(default)]
    pub stale_after_secs: Option<u64>,
}

fn default_api_profile_enabled() -> bool {
    true
}

/// Curated vault paths relative to chat `workspace_root`. Identity file paths trigger snapshot hot-reload; `99_USER_UPLOADED` is watched recursively (activity is logged, see `spawn_vault_identity_watch`).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct VaultWatchConfig {
    #[serde(default = "default_vault_watch_enabled")]
    pub enabled: bool,
    #[serde(default = "default_vault_watch_debounce_ms")]
    pub debounce_ms: u64,
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
        "00_Core/Identity.md".to_string(),
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
    /// Logical id (Qdrant collection `fcp_vault_{workspace}`, ephemeral snapshot suffix, etc.).
    /// **Not** joined onto paths for chat: the on-disk workspace is always [`Self::config_source_dir`].
    pub workspace: String,
    /// Carried in config for compatibility; chat and ignition use the current directory, not `vault_root` + `workspace`.
    pub vault_root: PathBuf,
    pub log_level: String,
    pub ollama_host: String,
    pub model_name: String,
    /// Human display name from `.fcp/config.toml` / `FCP_USER_NAME`; empty = unset.
    #[serde(default)]
    pub user_name: String,
    pub num_ctx: usize,
    pub generation_timeout_secs: u64,
    /// Maps to Ollama chat API `think`: `true` enables the thinking/reasoning path; `false` disables it.
    pub enable_reasoning_fsm: bool,
    pub condensation_threshold: f32,
    pub condensation_target: usize,
    pub max_tool_rounds: u8,
    pub max_recovery_attempts: u8,
    pub ephemeral_ttl_secs: u64,
    pub qdrant_url: String,
    #[serde(skip)] // Computed dynamically at runtime
    pub qdrant_collection: String,
    pub snapshot_interval_secs: u64,
    pub embed_model_name: String,
    pub idle_timeout_secs: u64,
    /// When true, spawn the idle monitor that may inject [`crate::executive::error::FcpError::Interrupted`] after [`Self::idle_timeout_secs`]. Default off until Gardener/sleep features land.
    #[serde(default)]
    pub idle_heartbeat_enabled: bool,
    pub web_fetch_timeout_secs: u64,
    pub web_fetch_max_bytes: usize,
    /// Fraction of [`Self::num_ctx`] used to cap vault read and web chunk sizes (tool budgets; not the condensation trigger).
    pub vault_read_ratio: f32,
    pub tool_match_threshold: f32,
    #[serde(default = "default_tool_descriptor_jit_top_k")]
    pub tool_descriptor_jit_top_k: usize,
    #[serde(default = "default_tool_descriptor_jit_max_chars")]
    pub tool_descriptor_jit_max_chars: usize,
    #[serde(default = "default_ollama_daemon")]
    pub ollama_daemon: DaemonCommand,
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
    #[serde(default)]
    pub apis: HashMap<String, ApiProfile>,
    #[serde(default)]
    pub vault_watch: VaultWatchConfig,
    /// When true, [`crate::orchestrator::context_view::build_llm_view`] feeds a lean copy to the LLM only; [`crate::orchestrator::core::Orchestrator::chat_stack`] stays full fidelity.
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
    /// When true, keep full JSON parameter schemas in the LLM view for tool definitions (larger prompt). When false and [`Self::optimize_context`] is true, [`crate::orchestrator::context_view::build_llm_view`] strips `parameters` in that block only; [`crate::orchestrator::core::Orchestrator::chat_stack`] stays full. Independently, the orchestrator forces full schemas for one recovery LLM pass after a Gatekeeper schema fault ([`crate::orchestrator::core::Orchestrator::force_full_tool_schemas_in_llm_view`]).
    #[serde(default = "default_optimize_context_full_tool_schemas")]
    pub optimize_context_full_tool_schemas: bool,
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
    /// Current working directory when [`AppConfig::load`] ran — this is the physical vault root for chat.
    #[serde(skip)]
    pub config_source_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DaemonCommand {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

fn default_ollama_daemon() -> DaemonCommand {
    DaemonCommand {
        command: "ollama".into(),
        args: vec!["serve".into()],
    }
}

fn default_qdrant_daemon() -> DaemonCommand {
    DaemonCommand {
        command: "qdrant".into(),
        args: Vec::new(),
    }
}

fn default_tool_descriptor_jit_top_k() -> usize {
    3
}

fn default_tool_descriptor_jit_max_chars() -> usize {
    6000
}

fn default_require_semantic_brain() -> bool {
    true
}

fn default_semantic_brain_connect_attempts() -> u32 {
    12
}

fn default_semantic_brain_connect_retry_delay_ms() -> u64 {
    500
}

fn default_optimize_context() -> bool {
    false
}

fn default_optimize_context_max_tool_snippet_chars() -> usize {
    400
}

fn default_optimize_context_assistant_compact() -> bool {
    true
}

fn default_optimize_context_full_tool_schemas() -> bool {
    false
}

fn default_memory_query_default_top_k() -> u32 {
    5
}

fn default_memory_query_top_k_max() -> u32 {
    25
}

fn default_memory_query_default_max_total_chars() -> u32 {
    10_000
}

fn default_memory_query_min_max_total_chars() -> u32 {
    256
}

fn default_memory_query_oversample_cap() -> u64 {
    200
}

fn default_memory_query_oversample_multiplier() -> u64 {
    25
}

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

fn default_builtin_apis() -> HashMap<String, ApiProfile> {
    let mut apis = default_open_meteo_apis();
    apis.extend(default_wikipedia_page_summary_api());
    apis
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            workspace: "default".into(),
            vault_root: PathBuf::from("./vaults/"),
            log_level: "info".into(),
            ollama_host: "http://localhost:11434".into(),
            model_name: "llama3.2".into(),
            user_name: String::new(),
            num_ctx: 8192,
            generation_timeout_secs: 120,
            enable_reasoning_fsm: true,
            condensation_threshold: 0.75,
            condensation_target: 300,
            max_tool_rounds: 5,
            max_recovery_attempts: 3,
            ephemeral_ttl_secs: 7200,
            qdrant_url: "http://localhost:6334".into(),
            qdrant_collection: "fcp_vault_default".into(),
            snapshot_interval_secs: 300,
            embed_model_name: "nomic-embed-text".into(),
            idle_timeout_secs: 900,
            idle_heartbeat_enabled: false,
            web_fetch_timeout_secs: 10,
            web_fetch_max_bytes: 20480,
            vault_read_ratio: 0.5,
            tool_match_threshold: 0.50,
            tool_descriptor_jit_top_k: 3,
            tool_descriptor_jit_max_chars: 6000,
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
            optimize_context_full_tool_schemas: default_optimize_context_full_tool_schemas(),
            memory_query_default_top_k: default_memory_query_default_top_k(),
            memory_query_top_k_max: default_memory_query_top_k_max(),
            memory_query_default_max_total_chars: default_memory_query_default_max_total_chars(),
            memory_query_min_max_total_chars: default_memory_query_min_max_total_chars(),
            memory_query_oversample_cap: default_memory_query_oversample_cap(),
            memory_query_oversample_multiplier: default_memory_query_oversample_multiplier(),
            memory_query_oversample_min: default_memory_query_oversample_min(),
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

        config.qdrant_collection = format!("fcp_vault_{}", config.workspace);

        Ok(config)
    }

    /// Physical directory for chat, ignition, tools, and `.fcp/` — always the process working directory
    /// at [`AppConfig::load`] (i.e. `cd` into your vault, then run `eris chat`).
    ///
    /// [`Self::workspace`] and [`Self::vault_root`] do **not** form this path; they remain logical / legacy config only.
    pub fn active_vault(&self) -> PathBuf {
        self.config_source_dir.clone()
    }

    /// Absolute paths under `chat_workspace_root` for vault watch (e.g. `00_Core/Identity.md`).
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
                command: Commands::Chat,
            };

            let config = AppConfig::load(cli).expect("Failed to load config");

            assert_eq!(config.workspace, "cli_workspace");
            assert_eq!(config.vault_root, PathBuf::from("/cli/vaults"));
            assert_eq!(config.log_level, "error");
            assert_eq!(config.qdrant_collection, "fcp_vault_cli_workspace");

            // Test fallback
            let cli2 = Cli {
                workspace: "default".to_string(),
                vault: None,
                verbose: 0,
                command: Commands::Chat,
            };

            let config2 = AppConfig::load(cli2).expect("Failed to load config");

            assert_eq!(config2.workspace, "env_workspace");
            assert_eq!(config2.vault_root, PathBuf::from("/toml/vaults"));
            assert_eq!(config2.log_level, "error");
            assert_eq!(config2.qdrant_collection, "fcp_vault_env_workspace");

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
            "ephemeral_ttl_secs": 3600,
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
        assert_eq!(parsed_config.ephemeral_ttl_secs, 3600);
        assert_eq!(parsed_config.qdrant_url, "http://localhost:6334");
        // qdrant_collection is skipped in serde, so it uses Default::default() if possible, but actually since we derive Deserialize, fields that are skipped will use their Default::default() type value which is String::default() i.e., "".
        assert_eq!(parsed_config.qdrant_collection, "");
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
}
