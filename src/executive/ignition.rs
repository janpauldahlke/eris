use crate::config::{AppConfig, LlmBackend};
use crate::executive::error::{FcpError, Result};
use std::path::Path;
use tokio::fs;
use inquire::{Select, Text};
use ollama_rs::Ollama;

/// Values fixed before interactive ignition (e.g. first-run welder).
#[derive(Debug, Clone)]
pub struct IgnitionOptions {
    pub workspace: String,
}

impl Default for IgnitionOptions {
    fn default() -> Self {
        Self {
            workspace: AppConfig::default().workspace,
        }
    }
}

pub async fn run_ignition_sequence(workspace_root: &Path, options: IgnitionOptions) -> Result<AppConfig> {
    const LLAMA_SERVER_DEFAULT_MODEL: &str = "qwen2.5-coder:14b";

    // 1. Fetch available models first to keep async cleanly separated
    let host = "http://localhost".to_string();
    let port = 11434;
    let client = Ollama::new(host, port);
    
    let local_models = client.list_local_models().await.ok().unwrap_or_default();
    let model_names: Vec<String> = local_models.into_iter().map(|m| m.name).collect();

    // 2. Interactive Prompts (blocking task)
    let (agent_name, user_name, llm_backend, model_name, constrained_protocol_enabled) =
        tokio::task::spawn_blocking(move || -> Result<(String, String, LlmBackend, String, bool)> {
        let agent_name = Text::new("Agent Name:")
            .with_default("ERIS")
            .prompt()
            .map_err(|e| match e {
                inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted => {
                    FcpError::Cancellation("Ignition cancelled by user".into())
                }
                _ => FcpError::Config(format!("Prompt error: {}", e)),
            })?;

        let user_name = Text::new("Your name (optional):")
            .with_default("")
            .prompt()
            .map_err(|e| match e {
                inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted => {
                    FcpError::Cancellation("Ignition cancelled by user".into())
                }
                _ => FcpError::Config(format!("Prompt error: {}", e)),
            })?;
        let user_name = user_name.trim().to_string();

        let backend_label = Select::new(
            "Chat Backend:",
            vec![
                "Local Ollama".to_string(),
                "llama-server (OpenAI-compatible)".to_string(),
            ],
        )
        .prompt()
        .map_err(|e| match e {
            inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted => {
                FcpError::Cancellation("Ignition cancelled by user".into())
            }
            _ => FcpError::Config(format!("Prompt error: {}", e)),
        })?;

        let (llm_backend, model_name, constrained_protocol_enabled) =
            if backend_label.starts_with("Local Ollama") {
                let model_name = if !model_names.is_empty() {
                    let default_idx = model_names
                        .iter()
                        .position(|m| m.contains("qwen2.5:14b"))
                        .unwrap_or(0);
                    Select::new("Ollama Model:", model_names.clone())
                        .with_starting_cursor(default_idx)
                        .prompt()
                        .map_err(|e| match e {
                            inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted => {
                                FcpError::Cancellation("Ignition cancelled by user".into())
                            }
                            _ => FcpError::Config(format!("Prompt error: {}", e)),
                        })?
                } else {
                    Text::new("Ollama Model:")
                        .with_default("qwen2.5:14b")
                        .prompt()
                        .map_err(|e| match e {
                            inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted => {
                                FcpError::Cancellation("Ignition cancelled by user".into())
                            }
                            _ => FcpError::Config(format!("Prompt error: {}", e)),
                        })?
                };
                (LlmBackend::Ollama, model_name, false)
            } else {
                (
                    LlmBackend::LlamaServer,
                    LLAMA_SERVER_DEFAULT_MODEL.to_string(),
                    true,
                )
            };

        Ok((
            agent_name,
            user_name,
            llm_backend,
            model_name,
            constrained_protocol_enabled,
        ))
    }).await.map_err(|e| FcpError::Config(format!("Spawn blocking failed: {}", e)))??;

    tracing::info!(
        backend = ?llm_backend,
        model = %model_name,
        constrained_protocol_enabled,
        "Ignition model/backend selection complete"
    );

    // 3. The Scaffold (v2 Zettelkasten roots)
    let dirs_to_create = [
        crate::vault_layout::telemetry_logs_dir(workspace_root),
        crate::vault_layout::tools_dir(workspace_root),
        workspace_root.join("00_Invariants"),
        workspace_root.join("10_Topology"),
        workspace_root.join("20_Discourse"),
        workspace_root.join("30_Synthesis"),
    ];

    for dir in &dirs_to_create {
        if !dir.exists() {
            fs::create_dir_all(dir).await?;
        }
    }

    // 4. The Cure (Identity Generation)
    let identity_path = workspace_root.join("00_Invariants/Identity.md");
    let mut identity_content = format!(
        "You are an autonomous AI operating in a strict state machine. You MUST communicate EXCLUSIVELY in JSON format matching the schemas provided. Never output plain conversational text.\n\nAgent Name: {} (this is you!)",
        agent_name
    );
    if !user_name.is_empty() {
        identity_content.push_str(&format!(
            "\nUser Name is: {} (your main user!)",
            user_name
        ));
    }
    fs::write(&identity_path, &identity_content).await?;

    fs::metadata(&identity_path).await.map_err(|e| {
        FcpError::WorkspaceFault {
            workspace: workspace_root.display().to_string(),
            reason: format!(
                "Identity.md missing after write (verify failed): {}: {}",
                identity_path.display(),
                e
            ),
        }
    })?;

    // 5. The Seal
    let mut config = AppConfig {
        model_name,
        llm_backend,
        constrained_protocol_enabled,
        user_name,
        workspace: options.workspace,
        ..Default::default()
    };
    config.qdrant_collection_v2 = format!("fcp_vault_v2_{}", config.workspace);
    
    let config_toml = toml::to_string(&config).map_err(|e| FcpError::Config(format!("Failed to serialize config: {}", e)))?;
    let config_path = crate::vault_layout::config_toml(workspace_root);
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::write(&config_path, config_toml).await?;

    let seal_path = crate::vault_layout::seal(workspace_root);
    let seal_content = format!(
        "agent={}\nmodel={}\nsealed_at={}",
        agent_name,
        config.model_name,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    );
    fs::write(&seal_path, seal_content).await?;

    Ok(config)
}
