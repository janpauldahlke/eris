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
            use std::collections::VecDeque;
            use crate::orchestrator::core::Orchestrator;
            use crate::engine::ollama::OllamaClient;
            use crate::memory::ephemeral::EphemeralMemory;
            use crate::tools::Gatekeeper;
            use std::path::PathBuf;
            use ollama_rs::Ollama;

            let workspace_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            // 1. Setup channels + terminal early so startup status is visible in TUI telemetry.
            let (tui_tx, tui_rx) = mpsc::channel(100);
            let (action_tx, mut action_rx) = mpsc::channel::<crate::ui::events::UserAction>(100);
            let terminal = setup_terminal()?;
            let _ = tui_tx
                .send(crate::ui::events::TuiEvent::SystemError(
                    "[startup] Checking peripheral daemons (Ollama, Qdrant)...".into(),
                ))
                .await;

            let mut peripheral_lifecycle =
                crate::executive::peripherals::ensure_peripherals_for_chat(&config).await?;
            let ollama_status = if peripheral_lifecycle.started_ollama() {
                "started by eris"
            } else {
                "already running"
            };
            let qdrant_status = if peripheral_lifecycle.started_qdrant() {
                "started by eris"
            } else {
                "already running"
            };
            let _ = tui_tx
                .send(crate::ui::events::TuiEvent::SystemError(format!(
                    "[startup] Peripheral readiness: ollama={ollama_status}, qdrant={qdrant_status}"
                )))
                .await;

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

            // 4. Build Engine
            let client = Ollama::new(host, port);
            let engine = OllamaClient::new(client.clone(), config.clone());
            let ollama_arc = Arc::new(client);
            let ephemeral = Arc::new(EphemeralMemory::new(config.workspace.clone()));
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
                        Some(semantic)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Semantic Brain offline. Vector tools will be unavailable.");
                        None
                    }
                };

            // 5. Register ALL tools with the Gatekeeper
            let mut gatekeeper = Gatekeeper::new();
            let read_limit = (config.llm_context_window as f32 * config.vault_read_ratio) as usize;
            let web_chunk_chars = read_limit.max(512);
            let web_preview_chars = (web_chunk_chars / 2).max(256);
            let effective_web_fetch_max_bytes = config
                .web_fetch_max_bytes
                .min(web_chunk_chars.saturating_mul(6))
                .max(web_chunk_chars);

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
                effective_web_fetch_max_bytes,
                web_chunk_chars,
                web_preview_chars,
                config.ephemeral_ttl_secs,
                ephemeral.clone(),
                semantic_arc.clone(),
            )));
            gatekeeper.register(Arc::new(crate::tools::web::WebArtifactQueryTool {
                ephemeral: ephemeral.clone(),
                semantic: semantic_arc.clone(),
                max_snippet_chars: (web_chunk_chars / 3).clamp(300, 900),
                max_total_chars: (web_chunk_chars / 2).clamp(1000, 2500),
            }));
            gatekeeper.register(Arc::new(crate::tools::system::SystemHealthTool));

            let max_content_chars = config.num_ctx * 3;
            gatekeeper.register(Arc::new(crate::tools::memory::MemoryStageTool {
                ephemeral: ephemeral.clone(),
                ttl_secs: config.ephemeral_ttl_secs,
                max_content_chars,
            }));
            gatekeeper.register(Arc::new(crate::tools::memory::MemoryStagedListTool {
                ephemeral: ephemeral.clone(),
            }));

            // Register memory tools if semantic backend is available
            if let Some(semantic) = &semantic_arc {
                gatekeeper.register(Arc::new(crate::tools::memory::MemoryCommitTool {
                    workspace_root: workspace_root.clone(),
                    semantic: semantic.clone(),
                    ephemeral: ephemeral.clone(),
                }));
                gatekeeper.register(Arc::new(crate::tools::memory::MemoryCommitAllTool {
                    workspace_root: workspace_root.clone(),
                    semantic: semantic.clone(),
                    ephemeral: ephemeral.clone(),
                }));
                gatekeeper.register(Arc::new(crate::tools::memory::MemoryQueryTool {
                    workspace: config.workspace.clone(),
                    semantic: semantic.clone(),
                }));
            }

            // 5c. Load tool descriptors (TOML) and enforce strict coverage.
            let descriptor_dir = workspace_root.join("tool_specs");
            let descriptor_registry = crate::tools::ToolDescriptorRegistry::load_from_dir(&descriptor_dir).await?;
            descriptor_registry.assert_covers_registered_tools(&gatekeeper.registered_tool_names())?;
            tracing::info!(
                descriptor_count = descriptor_registry.len(),
                dir = %descriptor_dir.display(),
                "Tool descriptor registry loaded"
            );
            let descriptor_registry = Arc::new(descriptor_registry);

            // 5b. Build ToolRouter (semantic tool gating via nomic embeddings)
            let tool_router = match crate::orchestrator::tool_router::ToolRouter::new(
                ollama_arc,
                config.embed_model_name.clone(),
                gatekeeper.all_tool_descriptions(),
                Some(descriptor_registry.clone()),
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
                interrupt_tx.clone(),
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
                config.tool_descriptor_jit_top_k,
                config.tool_descriptor_jit_max_chars,
                interrupt_rx,
                Some(tui_tx.clone()),
                tool_router,
                Some(descriptor_registry),
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
            let interrupt_tx_user = interrupt_tx.clone();
            tokio::spawn(async move {
                let mut pending_inputs: VecDeque<String> = VecDeque::new();
                loop {
                    tokio::select! {
                        Some(action) = action_rx.recv() => {
                            match action {
                                crate::ui::events::UserAction::CancelCurrentTurn => {
                                    tracing::info!("User requested cancel current turn");
                                    let _ = interrupt_tx_user.send(());
                                    let _ = tui_tx_err.send(crate::ui::events::TuiEvent::SystemError("[ui] Cancel requested".into())).await;
                                }
                                crate::ui::events::UserAction::Submit(msg) => {
                                    let trimmed = msg.trim().to_string();
                                    if trimmed.is_empty() {
                                        continue;
                                    }
                                    if pending_inputs.len() >= 3 {
                                        let _ = pending_inputs.pop_front();
                                        let _ = tui_tx_err.send(crate::ui::events::TuiEvent::SystemError("[ui] Queue full; dropped oldest queued input".into())).await;
                                    }
                                    pending_inputs.push_back(trimmed);
                                    if pending_inputs.len() > 1 {
                                        let _ = tui_tx_err.send(crate::ui::events::TuiEvent::SystemError(format!(
                                            "[ui] Processing older request ({} newer queued)",
                                            pending_inputs.len() - 1
                                        ))).await;
                                    }
                                }
                            }
                        }
                        _ = cancel_token_loop.cancelled() => {
                            tracing::info!("Orchestrator loop received cancellation signal");
                            break;
                        }
                    }

                    while let Some(msg) = pending_inputs.pop_front() {
                        orchestrator.queued_inputs = pending_inputs.len();
                        orchestrator.broadcast_state().await;
                        tracing::info!(msg_len = msg.len(), queued = pending_inputs.len(), "User input received");
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
                        orchestrator.queued_inputs = pending_inputs.len();
                        orchestrator.broadcast_state().await;
                    }
                }
            });

            // 10. Run TUI App
            let mut app = TuiApp::new(tui_rx, action_tx);
            let result = app.run(terminal).await;

            // 11. Teardown
            cancel_token.cancel();
            restore_terminal()?;
            eprintln!("[shutdown] Tearing down owned peripheral daemons...");
            let stopped = peripheral_lifecycle.shutdown_started_peripherals();
            if stopped.is_empty() {
                eprintln!("[shutdown] No peripheral daemons were started by this session.");
            } else {
                eprintln!("[shutdown] Stopped daemons: {}", stopped.join(", "));
            }
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
