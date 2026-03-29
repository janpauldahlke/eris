use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct AppConfig {
    pub workspace: String,
    pub vault_root: PathBuf,
    pub log_level: String,
    pub ollama_host: String,
    pub model_name: String,
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
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            workspace: "default".into(),
            vault_root: PathBuf::from("./vaults/"),
            log_level: "info".into(),
            ollama_host: "http://localhost:11434".into(),
            model_name: "llama3.2".into(),
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
            "embed_model_name": "nomic-embed-text"
        }"#;

        let parsed_config: AppConfig = serde_json::from_str(json_data).expect("Failed to parse JSON");

        assert_eq!(parsed_config.workspace, "test_workspace");
        assert_eq!(parsed_config.vault_root, PathBuf::from("/tmp/vaults"));
        assert_eq!(parsed_config.log_level, "debug");
        assert_eq!(parsed_config.ollama_host, "http://localhost:11434");
        assert_eq!(parsed_config.model_name, "qwen2.5:14b");
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
    }
}
