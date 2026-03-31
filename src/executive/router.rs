use crate::executive::cli::Commands;
use crate::executive::error::{FcpError, Result};
use crate::config::AppConfig;
use tokio_util::sync::CancellationToken;
use std::sync::Arc;

pub async fn execute_command(cmd: Commands, config: Arc<AppConfig>, cancel_token: CancellationToken) -> Result<()> {
    match cmd {
        Commands::Chat => {
            use crate::ui::terminal::{setup_terminal, restore_terminal};
            use crate::ui::TuiApp;
            use tokio::sync::mpsc;
            use crate::orchestrator::core::Orchestrator;
            use crate::engine::ollama::OllamaClient;
            use crate::memory::ephemeral::EphemeralMemory;
            use crate::tools::Gatekeeper;
            use std::path::PathBuf;
            use ollama_rs::Ollama;

            let workspace_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

            // 0. Ignition Sequence
            let seal_path = workspace_root.join(".fcp_seal");
            if !seal_path.exists() {
                crate::executive::ignition::run_ignition_sequence(&workspace_root).await?;
            }

            // 1. Parse Ollama host into components
            let parsed_url = url::Url::parse(&config.ollama_host)
                .map_err(|e| FcpError::Config(format!("Invalid ollama_host URL: {}", e)))?;
            let host = format!("{}://{}", parsed_url.scheme(), parsed_url.host_str().unwrap_or("localhost"));
            let port = parsed_url.port().unwrap_or(11434);

            // 2. Setup channels
            let (tui_tx, tui_rx) = mpsc::channel(100);
            let (action_tx, mut action_rx) = mpsc::channel::<String>(100);

            // 3. Setup Terminal
            let terminal = setup_terminal()?;

            // 4. Build Engine
            let client = Ollama::new(host, port);
            let engine = OllamaClient::new(client.clone(), config.clone());
            let ollama_arc = Arc::new(client);
            let ephemeral = Arc::new(EphemeralMemory::new(config.workspace.clone()));

            // 5. Register ALL tools with the Gatekeeper
            let mut gatekeeper = Gatekeeper::new();
            let read_limit = (config.llm_context_window as f32 * config.vault_read_ratio) as usize;

            gatekeeper.register(Arc::new(crate::tools::vault::VaultReadTool {
                workspace_root: workspace_root.clone(),
                read_limit,
            }));
            gatekeeper.register(Arc::new(crate::tools::vault::VaultWriteTool {
                workspace_root: workspace_root.clone(),
                max_content_chars: config.num_ctx * 3,
            }));
            gatekeeper.register(Arc::new(crate::tools::vault::VaultListTool {
                workspace_root: workspace_root.clone(),
            }));
            gatekeeper.register(Arc::new(crate::tools::agenda::AgendaPushTool {
                workspace_root: workspace_root.clone(),
            }));
            gatekeeper.register(Arc::new(crate::tools::agenda::AgendaListTool {
                workspace_root: workspace_root.clone(),
            }));
            gatekeeper.register(Arc::new(crate::tools::agenda::AgendaCompleteTool {
                workspace_root: workspace_root.clone(),
            }));
            gatekeeper.register(Arc::new(crate::tools::web::WebFetchTool::new(
                config.web_fetch_timeout_secs,
                config.web_fetch_max_bytes,
            )));
            gatekeeper.register(Arc::new(crate::tools::system::SystemHealthTool));

            let max_content_chars = config.num_ctx * 3;
            gatekeeper.register(Arc::new(crate::tools::memory::MemoryStageTool {
                ephemeral: ephemeral.clone(),
                ttl_secs: config.ephemeral_ttl_secs,
                max_content_chars,
            }));

            // Instantiate SemanticBrain and register memory tools
            let semantic_arc: Option<Arc<crate::memory::semantic::SemanticBrain>> =
                match crate::memory::semantic::SemanticBrain::new(config.clone(), ollama_arc.clone()).await {
                    Ok(semantic_brain) => {
                        let semantic = Arc::new(semantic_brain);
                        tracing::info!("Semantic Brain online. Vector tools registered.");

                        match semantic.ingest_vault(&workspace_root).await {
                            Ok(count) if count > 0 => {
                                tracing::info!(files = count, "Boot-time vault ingestion complete");
                            }
                            Ok(_) => {
                                tracing::debug!("No vault files to ingest at boot");
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "Boot-time vault ingestion failed");
                            }
                        }

                        gatekeeper.register(Arc::new(crate::tools::memory::MemoryCommitTool {
                            workspace_root: workspace_root.clone(),
                            semantic: semantic.clone(),
                            ephemeral: ephemeral.clone(),
                        }));
                        gatekeeper.register(Arc::new(crate::tools::memory::MemoryQueryTool {
                            workspace: config.workspace.clone(),
                            semantic: semantic.clone(),
                        }));
                        Some(semantic)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Semantic Brain offline. Vector tools will be unavailable.");
                        None
                    }
                };

            // 5b. Build ToolRouter (semantic tool gating via nomic embeddings)
            let tool_router = match crate::orchestrator::tool_router::ToolRouter::new(
                ollama_arc,
                config.embed_model_name.clone(),
                gatekeeper.all_tool_descriptions(),
                config.tool_match_threshold,
            )
            .await
            {
                Ok(r) => {
                    tracing::info!("ToolRouter online. Semantic tool gating active.");
                    Some(r)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "ToolRouter offline — all requests will include tool schemas.");
                    None
                }
            };

            // 6. Heartbeat + Interrupt wiring
            let (interrupt_tx, interrupt_rx) = tokio::sync::watch::channel(());
            let last_input_time = Arc::new(std::sync::atomic::AtomicU64::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            ));

            crate::orchestrator::heartbeat::spawn_heartbeat_monitor(
                last_input_time.clone(),
                config.idle_timeout_secs,
                interrupt_tx,
                cancel_token.clone(),
            );

            // 7. Snapshot + promotion daemon
            crate::memory::ephemeral::spawn_snapshot_daemon(
                ephemeral.clone(),
                workspace_root.clone(),
                semantic_arc,
                config.snapshot_interval_secs,
                cancel_token.clone(),
            );

            // 8. Build Orchestrator
            // Pass workspace_root directly as vault_root with empty workspace string
            // so ContextAssembler resolves 00_Core at workspace_root/00_Core (not workspace_root/default/00_Core)
            let mut orchestrator = Orchestrator::new(
                engine,
                gatekeeper,
                ephemeral,
                &workspace_root,
                "",
                config.max_recovery_attempts,
                config.max_tool_rounds,
                config.condensation_threshold,
                config.num_ctx,
                interrupt_rx,
                Some(tui_tx.clone()),
                tool_router,
            );

            tracing::info!(
                model = %config.model_name,
                num_ctx = config.num_ctx,
                max_tool_rounds = config.max_tool_rounds,
                max_recovery = config.max_recovery_attempts,
                "Orchestrator initialized"
            );

            // 9. Spawn orchestrator loop
            let tui_tx_err = tui_tx.clone();
            let cancel_token_loop = cancel_token.clone();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        Some(msg) = action_rx.recv() => {
                            tracing::info!(msg_len = msg.len(), "User input received");
                            last_input_time.store(
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs(),
                                std::sync::atomic::Ordering::Relaxed,
                            );
                            orchestrator.chat_stack.push(crate::engine::Message {
                                role: "user".to_string(),
                                content: msg,
                            });
                            orchestrator.state = crate::orchestrator::state::AgentState::Chat;
                            if let Err(e) = orchestrator.step(None).await {
                                if matches!(e, FcpError::Interrupted) {
                                    tracing::info!("Orchestrator interrupted by heartbeat, continuing loop");
                                    continue;
                                }
                                let err_msg = format!("[FATAL ERROR] Orchestrator halted: {}", e);
                                tracing::error!(error = %e, "Orchestrator fatal error");
                                let _ = tui_tx_err.send(crate::ui::events::TuiEvent::SystemError(err_msg)).await;
                                break;
                            }
                        }
                        _ = cancel_token_loop.cancelled() => {
                            tracing::info!("Orchestrator loop received cancellation signal");
                            break;
                        }
                    }
                }
            });

            // 10. Run TUI App
            let mut app = TuiApp::new(tui_rx, action_tx);
            let result = app.run(terminal).await;

            // 11. Teardown
            cancel_token.cancel();
            restore_terminal()?;
            result
        }
        Commands::Run { prompt } => {
            let _ = prompt;
            Ok(())
        }
        Commands::Tool { name, args } => {
            let _ = args;
            match name.as_str() {
                "memory:query" => Ok(()),
                _ => Err(FcpError::Config(format!("Tool not found: {}", name))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn test_config() -> Arc<AppConfig> {
        Arc::new(AppConfig::default())
    }

    #[test]
    fn test_tool_non_existent_routing() {
        let cmd = Commands::Tool {
            name: "non_existent_tool".to_string(),
            args: "{}".to_string(),
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(execute_command(cmd, test_config(), CancellationToken::new()));
        
        assert!(result.is_err());
        match result.unwrap_err() {
            FcpError::Config(msg) => {
                assert!(msg.contains("non_existent_tool"));
            }
            _ => panic!("Expected Config error for non-existent tool"),
        }
    }

    #[tokio::test]
    async fn test_cancellation_token_yields() {
        let cancel_token = CancellationToken::new();
        let token_clone = cancel_token.clone();

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            token_clone.cancel();
        });

        cancel_token.cancelled().await;
        assert!(cancel_token.is_cancelled());
        
        let cmd = Commands::Run { prompt: "test".to_string() };
        let result = execute_command(cmd, test_config(), cancel_token).await;
        assert!(result.is_ok());
    }
}
