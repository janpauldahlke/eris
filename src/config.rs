use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct AppConfig {
    pub workspace: String,
    pub vault_root: PathBuf,
    pub log_level: String,
    pub ollama_host: String,
    pub model_name: String,
    /// Human display name from fcp.toml / `FCP_USER_NAME`; empty = unset.
    #[serde(default)]
    pub user_name: String,
    pub num_ctx: usize,
    pub generation_timeout_secs: u64,
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
    pub web_fetch_timeout_secs: u64,
    pub web_fetch_max_bytes: usize,
    pub llm_context_window: usize,
    pub vault_read_ratio: f32,
    pub tool_match_threshold: f32,
    /// Max number of tools that receive full JSON schemas in Tier 1 (semantic Top-K).
    #[serde(default = "default_tool_schema_top_k")]
    pub tool_schema_top_k: usize,
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

fn default_tool_schema_top_k() -> usize {
    3
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
            web_fetch_timeout_secs: 10,
            web_fetch_max_bytes: 20480,
            llm_context_window: 16384,
            vault_read_ratio: 0.25,
            tool_match_threshold: 0.50,
            tool_schema_top_k: 3,
            tool_descriptor_jit_top_k: 3,
            tool_descriptor_jit_max_chars: 6000,
            ollama_daemon: default_ollama_daemon(),
            qdrant_daemon: default_qdrant_daemon(),
            require_semantic_brain: default_require_semantic_brain(),
            semantic_brain_connect_attempts: default_semantic_brain_connect_attempts(),
            semantic_brain_connect_retry_delay_ms: default_semantic_brain_connect_retry_delay_ms(),
        }
    }
}

impl AppConfig {
    pub fn load(cli: crate::executive::cli::Cli) -> crate::executive::error::Result<Self> {
        use figment::{Figment, providers::{Env, Format, Toml}};

        let _ = dotenvy::dotenv();

        let figment = Figment::from(figment::providers::Serialized::defaults(AppConfig::default()))
            .merge(Toml::file("fcp.toml"))
            .merge(Env::prefixed("FCP_"));

        let mut config: AppConfig = figment.extract().map_err(|e| crate::executive::error::FcpError::Config(e.to_string()))?;

        if cli.workspace != "default" {
            config.workspace = cli.workspace;
        }

        if let Some(vault) = cli.vault {
            config.vault_root = vault;
        }

        config.qdrant_collection = format!("fcp_vault_{}", config.workspace);

        Ok(config)
    }

    pub fn active_vault(&self) -> PathBuf {
        self.vault_root.join(&self.workspace)
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
            jail.create_file("fcp.toml", r#"
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
            "llm_context_window": 16384,
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
        assert_eq!(parsed_config.llm_context_window, 16384);
        assert_eq!(parsed_config.vault_read_ratio, 0.25);
        assert_eq!(parsed_config.tool_match_threshold, 0.50);
        assert_eq!(parsed_config.ollama_daemon.command, "ollama");
        assert_eq!(parsed_config.ollama_daemon.args, vec!["serve"]);
        assert_eq!(parsed_config.qdrant_daemon.command, "qdrant");
        assert!(parsed_config.qdrant_daemon.args.is_empty());
    }
}
