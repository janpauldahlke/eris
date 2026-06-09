use crate::config::{default_llamacpp_ready_timeout, AppConfig, LlamaCppConfig, LlmBackend};
use crate::executive::error::{FcpError, Result};
use inquire::{Select, Text};
use ollama_rs::Ollama;
use std::path::{Path, PathBuf};
use tokio::fs;

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

pub async fn run_ignition_sequence(
    workspace_root: &Path,
    options: IgnitionOptions,
) -> Result<AppConfig> {
    // 1. Fetch available models first to keep async cleanly separated
    let host = "http://localhost".to_string();
    let port = 11434;
    let client = Ollama::new(host, port);

    let local_models = client.list_local_models().await.ok().unwrap_or_default();
    let model_names: Vec<String> = local_models.into_iter().map(|m| m.name).collect();

    // 2. Interactive Prompts (blocking task)
    #[derive(Debug)]
    struct IgnitionAnswers {
        agent_name: String,
        user_name: String,
        llm_backend: LlmBackend,
        model_name: String,
        ollama_num_gpu: Option<u32>,
        ollama_main_gpu: Option<u32>,
        ollama_low_vram: Option<bool>,
        llama_cpp: Option<LlamaCppConfig>,
        num_ctx: usize,
    }

    let answers = tokio::task::spawn_blocking(move || -> Result<IgnitionAnswers> {
        let agent_name = Text::new("Agent Name:")
            .with_default("ERIS")
            .prompt()
            .map_err(|e| match e {
                inquire::InquireError::OperationCanceled
                | inquire::InquireError::OperationInterrupted => {
                    FcpError::Cancellation("Ignition cancelled by user".into())
                }
                _ => FcpError::Config(format!("Prompt error: {}", e)),
            })?;

        let user_name = Text::new("Your name (optional):")
            .with_default("")
            .prompt()
            .map_err(|e| match e {
                inquire::InquireError::OperationCanceled
                | inquire::InquireError::OperationInterrupted => {
                    FcpError::Cancellation("Ignition cancelled by user".into())
                }
                _ => FcpError::Config(format!("Prompt error: {}", e)),
            })?;
        let user_name = user_name.trim().to_string();

        let backend_options = vec!["Ollama", "llama.cpp"];
        let backend_choice = Select::new("Backend:", backend_options)
            .prompt()
            .map_err(|e| match e {
                inquire::InquireError::OperationCanceled
                | inquire::InquireError::OperationInterrupted => {
                    FcpError::Cancellation("Ignition cancelled by user".into())
                }
                _ => FcpError::Config(format!("Prompt error: {}", e)),
            })?;

        let llm_backend = match backend_choice {
            "llama.cpp" => LlmBackend::LlamaCpp,
            _ => LlmBackend::Ollama,
        };

        match llm_backend {
            LlmBackend::Ollama => {
                let model_name = if !model_names.is_empty() {
                    let default_idx = model_names
                        .iter()
                        .position(|m| m.contains("qwen2.5:14b"))
                        .unwrap_or(0);
                    Select::new("Ollama Model:", model_names.clone())
                        .with_starting_cursor(default_idx)
                        .prompt()
                        .map_err(|e| match e {
                            inquire::InquireError::OperationCanceled
                            | inquire::InquireError::OperationInterrupted => {
                                FcpError::Cancellation("Ignition cancelled by user".into())
                            }
                            _ => FcpError::Config(format!("Prompt error: {}", e)),
                        })?
                } else {
                    Text::new("Ollama Model:")
                        .with_default("qwen2.5:14b")
                        .prompt()
                        .map_err(|e| match e {
                            inquire::InquireError::OperationCanceled
                            | inquire::InquireError::OperationInterrupted => {
                                FcpError::Cancellation("Ignition cancelled by user".into())
                            }
                            _ => FcpError::Config(format!("Prompt error: {}", e)),
                        })?
                };

                let ollama_low_vram = Some(
                    inquire::Confirm::new("Enable Ollama low VRAM mode?")
                        .with_default(false)
                        .prompt()
                        .map_err(|e| match e {
                            inquire::InquireError::OperationCanceled
                            | inquire::InquireError::OperationInterrupted => {
                                FcpError::Cancellation("Ignition cancelled by user".into())
                            }
                            _ => FcpError::Config(format!("Prompt error: {}", e)),
                        })?,
                );

                let num_gpu_raw = Text::new("Ollama num_gpu (GPU layers, blank = auto):")
                    .with_default("")
                    .prompt()
                    .map_err(|e| match e {
                        inquire::InquireError::OperationCanceled
                        | inquire::InquireError::OperationInterrupted => {
                            FcpError::Cancellation("Ignition cancelled by user".into())
                        }
                        _ => FcpError::Config(format!("Prompt error: {}", e)),
                    })?;
                let num_gpu_raw = num_gpu_raw.trim();
                let ollama_num_gpu = if num_gpu_raw.is_empty() {
                    None
                } else {
                    Some(num_gpu_raw.parse::<u32>().map_err(|_| {
                        FcpError::Config(
                            "Invalid num_gpu: expected non-negative integer".into(),
                        )
                    })?)
                };

                let main_gpu_raw = Text::new("Ollama main_gpu index (blank = default):")
                    .with_default("")
                    .prompt()
                    .map_err(|e| match e {
                        inquire::InquireError::OperationCanceled
                        | inquire::InquireError::OperationInterrupted => {
                            FcpError::Cancellation("Ignition cancelled by user".into())
                        }
                        _ => FcpError::Config(format!("Prompt error: {}", e)),
                    })?;
                let main_gpu_raw = main_gpu_raw.trim();
                let ollama_main_gpu = if main_gpu_raw.is_empty() {
                    None
                } else {
                    Some(main_gpu_raw.parse::<u32>().map_err(|_| {
                        FcpError::Config(
                            "Invalid main_gpu: expected non-negative integer".into(),
                        )
                    })?)
                };

                Ok(IgnitionAnswers {
                    agent_name,
                    user_name,
                    llm_backend,
                    model_name,
                    ollama_num_gpu,
                    ollama_main_gpu,
                    ollama_low_vram,
                    llama_cpp: None,
                    num_ctx: AppConfig::default().num_ctx,
                })
            }
            LlmBackend::LlamaCpp => {
                let home = loop {
                    let raw = Text::new("llama.cpp build directory:")
                        .with_default("~/llama.cpp/build")
                        .prompt()
                        .map_err(|e| match e {
                            inquire::InquireError::OperationCanceled
                            | inquire::InquireError::OperationInterrupted => {
                                FcpError::Cancellation("Ignition cancelled by user".into())
                            }
                            _ => FcpError::Config(format!("Prompt error: {}", e)),
                        })?;
                    let expanded = shellexpand::tilde(raw.trim()).to_string();
                    let path = PathBuf::from(&expanded);
                    let server_bin = path.join("bin").join("llama-server");
                    if server_bin.exists() {
                        break path;
                    }
                    eprintln!(
                        "  ✗ llama-server not found at {}\n    Please provide the build directory containing bin/llama-server.",
                        server_bin.display()
                    );
                };

                let chat_model_path = loop {
                    let raw = Text::new("Chat model GGUF path:")
                        .prompt()
                        .map_err(|e| match e {
                            inquire::InquireError::OperationCanceled
                            | inquire::InquireError::OperationInterrupted => {
                                FcpError::Cancellation("Ignition cancelled by user".into())
                            }
                            _ => FcpError::Config(format!("Prompt error: {}", e)),
                        })?;
                    let expanded = shellexpand::tilde(raw.trim()).to_string();
                    let path = PathBuf::from(&expanded);
                    if path.exists() && path.extension().is_some_and(|e| e == "gguf") {
                        break path;
                    }
                    eprintln!("  ✗ File not found or not a .gguf: {}", path.display());
                };

                let default_embed =
                    home.parent().unwrap_or(&home).join("models/nomic-embed-text-v1.5.Q8_0.gguf");
                let default_embed_str = default_embed.to_string_lossy().to_string();
                let embed_model_path = loop {
                    let raw = Text::new("Embed model GGUF path:")
                        .with_default(&default_embed_str)
                        .prompt()
                        .map_err(|e| match e {
                            inquire::InquireError::OperationCanceled
                            | inquire::InquireError::OperationInterrupted => {
                                FcpError::Cancellation("Ignition cancelled by user".into())
                            }
                            _ => FcpError::Config(format!("Prompt error: {}", e)),
                        })?;
                    let expanded = shellexpand::tilde(raw.trim()).to_string();
                    let path = PathBuf::from(&expanded);
                    if path.exists() {
                        break path;
                    }
                    eprintln!("  ✗ File not found: {}", path.display());
                };

                let default_num_ctx = AppConfig::default().num_ctx.to_string();
                let num_ctx_raw = Text::new(
                    "Context window (num_ctx: orchestrator + managed llama-server --ctx-size):",
                )
                .with_default(&default_num_ctx)
                .prompt()
                .map_err(|e| match e {
                        inquire::InquireError::OperationCanceled
                        | inquire::InquireError::OperationInterrupted => {
                            FcpError::Cancellation("Ignition cancelled by user".into())
                        }
                        _ => FcpError::Config(format!("Prompt error: {}", e)),
                    })?;
                let num_ctx: usize = num_ctx_raw.trim().parse().map_err(|_| {
                    FcpError::Config("Invalid num_ctx: expected positive integer".into())
                })?;
                let num_ctx = num_ctx.max(1);

                let gpu_raw = Text::new("GPU layers (--n-gpu-layers, 0 = CPU only):")
                    .with_default("99")
                    .prompt()
                    .map_err(|e| match e {
                        inquire::InquireError::OperationCanceled
                        | inquire::InquireError::OperationInterrupted => {
                            FcpError::Cancellation("Ignition cancelled by user".into())
                        }
                        _ => FcpError::Config(format!("Prompt error: {}", e)),
                    })?;
                let n_gpu_layers: u32 = gpu_raw.trim().parse().map_err(|_| {
                    FcpError::Config(
                        "Invalid n_gpu_layers: expected non-negative integer".into(),
                    )
                })?;

                let llama_cpp_config = LlamaCppConfig {
                    home,
                    chat_server_url: "http://127.0.0.1:8090".into(),
                    embed_server_url: "http://127.0.0.1:8091".into(),
                    chat_model_path,
                    embed_model_path,
                    n_gpu_layers,
                    ready_timeout_secs: default_llamacpp_ready_timeout(),
                    detach_servers_on_chat_exit: false,
                    shutdown_grace_secs: 30,
                    shutdown_stagger_secs: 3,
                    shutdown_allow_sigkill: true,
                    mmproj_path: None,
                    media_path: None,
                };

                Ok(IgnitionAnswers {
                    agent_name,
                    user_name,
                    llm_backend,
                    model_name: "llama-cpp-local".into(),
                    ollama_num_gpu: None,
                    ollama_main_gpu: None,
                    ollama_low_vram: None,
                    llama_cpp: Some(llama_cpp_config),
                    num_ctx,
                })
            }
        }
    })
    .await
    .map_err(|e| FcpError::Config(format!("Spawn blocking failed: {}", e)))??;

    let IgnitionAnswers {
        agent_name,
        user_name,
        llm_backend,
        model_name,
        ollama_num_gpu,
        ollama_main_gpu,
        ollama_low_vram,
        llama_cpp,
        num_ctx,
    } = answers;

    // 3. The Scaffold (v2 Zettelkasten roots)
    let dirs_to_create = [
        crate::vault_layout::telemetry_logs_dir(workspace_root),
        crate::vault_layout::tools_dir(workspace_root),
        workspace_root.join("00_Invariants"),
        workspace_root.join("10_Topology"),
        workspace_root.join("20_Discourse"),
        workspace_root.join("20_Discourse/web/missions"),
        workspace_root.join("30_Synthesis"),
        workspace_root.join("40_MEDIA"),
    ];

    for dir in &dirs_to_create {
        if !dir.exists() {
            fs::create_dir_all(dir).await?;
        }
    }

    crate::tools::web::bootstrap::seed_web_operator_files(workspace_root).await?;

    // Seed default runtime skills into the workspace vault (seed-only; never overwrite).
    let seed_report = crate::skills::seed_runtime_skills(workspace_root).await?;
    tracing::info!(
        copied = seed_report.copied,
        skipped_existing = seed_report.skipped_existing,
        "Runtime skills seeded during ignition"
    );

    // 4. The Cure (Identity Generation)
    let identity_path = workspace_root.join("00_Invariants/Identity.md");
    let mut identity_content = format!(
        "You are an autonomous AI operating in a strict state machine. You MUST communicate EXCLUSIVELY in JSON format matching the schemas provided. Never output plain conversational text.\n\nAgent Name: {} (this is you!)",
        agent_name
    );
    if !user_name.is_empty() {
        identity_content.push_str(&format!("\nUser Name is: {} (your main user!)", user_name));
    }
    fs::write(&identity_path, &identity_content).await?;

    fs::metadata(&identity_path)
        .await
        .map_err(|e| FcpError::WorkspaceFault {
            workspace: workspace_root.display().to_string(),
            reason: format!(
                "Identity.md missing after write (verify failed): {}: {}",
                identity_path.display(),
                e
            ),
        })?;

    // 5. The Seal
    let mut config = AppConfig {
        model_name,
        llm_backend,
        user_name,
        num_ctx,
        ollama_num_gpu,
        ollama_main_gpu,
        ollama_low_vram,
        llama_cpp,
        workspace: options.workspace,
        ..Default::default()
    };
    config.qdrant_collection_v2 = format!("fcp_vault_v2_{}", config.workspace);

    let config_toml = toml::to_string(&config)
        .map_err(|e| FcpError::Config(format!("Failed to serialize config: {}", e)))?;
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

