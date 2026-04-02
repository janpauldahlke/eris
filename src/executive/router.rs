use crate::executive::cli::{Cli, Commands};
use crate::executive::error::{FcpError, Result};
use crate::config::AppConfig;
use tokio_util::sync::CancellationToken;
use std::sync::Arc;

pub async fn execute_command(cli: Cli, config: Arc<AppConfig>, cancel_token: CancellationToken) -> Result<()> {
    match cli.command {
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

            let mut config = config;
            let seal_path = workspace_root.join(".fcp_seal");
            if !seal_path.exists() {
                crate::executive::ignition::run_ignition_sequence(&workspace_root).await?;
                config = Arc::new(AppConfig::load(cli.clone())?);
            }
            crate::executive::identity_md::sync_identity_user_line(&workspace_root, &config.user_name)
                .await?;

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

            // 1. Parse Ollama host into components
            let parsed_url = url::Url::parse(&config.ollama_host)
                .map_err(|e| FcpError::Config(format!("Invalid ollama_host URL: {}", e)))?;
            let host = format!("{}://{}", parsed_url.scheme(), parsed_url.host_str().unwrap_or("localhost"));
            let port = parsed_url.port().unwrap_or(11434);

            // 4. Build Engine + shared last-token snapshot (watch channel; see `engine::token_metrics`)
            let client = Ollama::new(host, port);
            let (token_metrics_tx, token_metrics_rx) = crate::engine::token_metrics::channel();
            let engine = OllamaClient::with_token_metrics(client.clone(), config.clone(), token_metrics_tx);
            let ollama_arc = Arc::new(client);
            let ephemeral = Arc::new(EphemeralMemory::new(config.workspace.clone()));
            let connect_attempts = config.semantic_brain_connect_attempts;
            let connect_retry_ms = config.semantic_brain_connect_retry_delay_ms;
            let semantic_arc: Option<Arc<crate::memory::semantic::SemanticBrain>> = match crate::memory::semantic::SemanticBrain::new_with_connect_retries(
                config.clone(),
                ollama_arc.clone(),
                connect_attempts,
                connect_retry_ms,
            )
            .await
            {
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
                    if config.require_semantic_brain {
                        return Err(FcpError::VectorDbOffline(format!(
                            "require_semantic_brain enabled: Qdrant gRPC did not come up after {connect_attempts} attempt(s): {e}"
                        )));
                    }
                    tracing::warn!(
                        error = %e,
                        attempts = connect_attempts,
                        "Semantic Brain offline after retries. Vector tools will be unavailable."
                    );
                    None
                }
            };

            // 5. Register ALL tools with the Gatekeeper
            let mut gatekeeper = Gatekeeper::new();
            let (alarm_reschedule_tx, alarm_reschedule_rx) =
                tokio::sync::mpsc::unbounded_channel::<()>();
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
            gatekeeper.register(Arc::new(crate::tools::agenda::AgendaRemindAtTool {
                workspace_root: workspace_root.clone(),
                reschedule_tx: alarm_reschedule_tx.clone(),
            }));
            gatekeeper.register(Arc::new(crate::tools::agenda::AgendaCompleteTool {
                workspace_root: workspace_root.clone(),
                reschedule_tx: alarm_reschedule_tx.clone(),
            }));
            gatekeeper.register(Arc::new(crate::tools::agenda::AgendaRemoveTool {
                workspace_root: workspace_root.clone(),
                reschedule_tx: alarm_reschedule_tx.clone(),
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

            gatekeeper.register(Arc::new(crate::tools::clock::ClockNowTool));
            gatekeeper.register(Arc::new(crate::tools::clock::ClockTimerTool {
                workspace_root: workspace_root.clone(),
                reschedule_tx: alarm_reschedule_tx.clone(),
            }));
            gatekeeper.register(Arc::new(crate::tools::clock::ClockWallAlarmTool {
                workspace_root: workspace_root.clone(),
                reschedule_tx: alarm_reschedule_tx,
            }));

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

            // 5c. Load compile-time embedded tool descriptors.
            // Runtime users cannot alter descriptor behavior without recompiling.
            let descriptor_registry = {
                let registry = crate::tools::ToolDescriptorRegistry::load_embedded()?;
                registry.assert_covers_registered_tools(&gatekeeper.registered_tool_names())?;
                tracing::info!(
                    descriptor_count = registry.len(),
                    "Embedded tool descriptor registry loaded"
                );
                Some(Arc::new(registry))
            };

            // 5b. Build ToolRouter (semantic tool gating via nomic embeddings)
            let tool_router = match crate::orchestrator::tool_router::ToolRouter::new(
                ollama_arc,
                config.embed_model_name.clone(),
                gatekeeper.all_tool_descriptions(),
                descriptor_registry.clone(),
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

            crate::orchestrator::alarm_scheduler::spawn_alarm_scheduler(
                workspace_root.clone(),
                tui_tx.clone(),
                alarm_reschedule_rx,
                cancel_token.clone(),
            );

            let startup_wp = workspace_root.clone();
            let startup_tui = tui_tx.clone();
            tokio::spawn(async move {
                if let Some(msg) =
                    crate::orchestrator::missed_agenda::startup_overdue_agenda_hint(&startup_wp).await
                {
                    let _ = startup_tui
                        .send(crate::ui::events::TuiEvent::SystemError(msg))
                        .await;
                }
            });

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
                descriptor_registry,
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
                                crate::ui::events::UserAction::SystemInject(label) => {
                                    let trimmed = label.trim().to_string();
                                    if trimmed.is_empty() {
                                        continue;
                                    }
                                    let content = format!(
                                        "{}{}",
                                        crate::ui::events::SYSTEM_ALARM_PREFIX,
                                        trimmed
                                    );
                                    orchestrator.chat_stack.push(crate::engine::Message {
                                        role: "user".to_string(),
                                        content,
                                    });
                                    orchestrator.state = crate::orchestrator::state::AgentState::Chat;
                                    last_input_time.store(
                                        std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .map(|d| d.as_secs())
                                            .unwrap_or(0),
                                        std::sync::atomic::Ordering::Relaxed,
                                    );
                                    orchestrator.broadcast_state().await;
                                    tracing::info!("SystemInject alarm turn");
                                    if let Err(e) = orchestrator.step(None).await {
                                        if matches!(e, FcpError::Interrupted) {
                                            tracing::info!("Orchestrator interrupted during alarm turn");
                                            continue;
                                        }
                                        let err_msg = format!("[FATAL ERROR] Orchestrator halted: {}", e);
                                        tracing::error!(error = %e, "Orchestrator fatal error");
                                        let _ = tui_tx_err.send(crate::ui::events::TuiEvent::SystemError(err_msg)).await;
                                        break;
                                    }
                                    orchestrator.broadcast_state().await;
                                }
                                crate::ui::events::UserAction::AgendaAlarmPending {
                                    agenda_task_id,
                                    label,
                                    alarm_record_id,
                                    seconds_late,
                                } => {
                                    let trimmed = label.trim().to_string();
                                    if trimmed.is_empty() {
                                        continue;
                                    }
                                    let late_note = if seconds_late > 60 {
                                        format!(" (~{} min late)", seconds_late / 60)
                                    } else {
                                        String::new()
                                    };
                                    let content = format!(
                                        "{}{}{}\n\n\
This is a linked agenda reminder — please answer explicitly:\n\
• Done — you finished this task now. Say clearly (e.g. \"done\" or \"finished\") so the assistant can mark it complete with agenda:complete.\n\
• Snooze — you still need a later nudge. Say when (e.g. \"in 10 minutes\" or \"at 15:00\") so the assistant can reschedule with agenda:remind_at using task_id below.\n\n\
[AGENDA_CONFIRM task_id={} alarm_id={} late_sec={}]",
                                        crate::ui::events::SYSTEM_ALARM_PREFIX,
                                        trimmed,
                                        late_note,
                                        agenda_task_id,
                                        alarm_record_id,
                                        seconds_late
                                    );
                                    orchestrator.chat_stack.push(crate::engine::Message {
                                        role: "user".to_string(),
                                        content,
                                    });
                                    orchestrator.state = crate::orchestrator::state::AgentState::Chat;
                                    last_input_time.store(
                                        std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .map(|d| d.as_secs())
                                            .unwrap_or(0),
                                        std::sync::atomic::Ordering::Relaxed,
                                    );
                                    orchestrator.broadcast_state().await;
                                    tracing::info!("Agenda-linked alarm turn");
                                    if let Err(e) = orchestrator.step(None).await {
                                        if matches!(e, FcpError::Interrupted) {
                                            tracing::info!("Orchestrator interrupted during alarm turn");
                                            continue;
                                        }
                                        let err_msg = format!("[FATAL ERROR] Orchestrator halted: {}", e);
                                        tracing::error!(error = %e, "Orchestrator fatal error");
                                        let _ = tui_tx_err.send(crate::ui::events::TuiEvent::SystemError(err_msg)).await;
                                        break;
                                    }
                                    orchestrator.broadcast_state().await;
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
            let result = app.run(terminal, Some(token_metrics_rx)).await;

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
    use crate::executive::cli::Cli;
    use std::time::Duration;

    fn test_config() -> Arc<AppConfig> {
        Arc::new(AppConfig::default())
    }

    fn test_cli(command: Commands) -> Cli {
        Cli {
            workspace: "default".to_string(),
            vault: None,
            verbose: 0,
            command,
        }
    }

    #[test]
    fn test_tool_non_existent_routing() {
        let cmd = Commands::Tool {
            name: "non_existent_tool".to_string(),
            args: "{}".to_string(),
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(execute_command(
            test_cli(cmd),
            test_config(),
            CancellationToken::new(),
        ));
        
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
        let result = execute_command(test_cli(cmd), test_config(), cancel_token).await;
        assert!(result.is_ok());
    }

    /// Submit queues work that runs `system:health` (Reflect), then `SystemInject` is already on
    /// `action_rx`. The first `step` must fully finish (tool + follow-up generation) before the
    /// relay pulls the alarm—FIFO on the single action channel.
    #[tokio::test]
    async fn relay_submit_then_system_inject_orders_after_tool() {
        use std::collections::VecDeque;
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
        use std::sync::Arc;

        use async_trait::async_trait;
        use tokio::sync::mpsc;

        use crate::engine::{EngineResponse, LlmEngine, Message};
        use crate::memory::ephemeral::EphemeralMemory;
        use crate::orchestrator::core::Orchestrator;
        use crate::orchestrator::state::AgentState;
        use crate::tools::gatekeeper::Gatekeeper;
        use crate::tools::system::SystemHealthTool;
        use crate::ui::events::{UserAction, SYSTEM_ALARM_PREFIX};

        #[derive(Clone)]
        struct SeqEngine {
            responses: Arc<Vec<String>>,
            calls: Arc<AtomicUsize>,
        }

        #[async_trait]
        impl LlmEngine for SeqEngine {
            async fn generate(
                &self,
                _stack: &[Message],
                _available_tools_json: &str,
                _stream_tx: Option<tokio::sync::mpsc::UnboundedSender<String>>,
            ) -> crate::executive::error::Result<EngineResponse> {
                let i = self.calls.fetch_add(1, AtomicOrdering::SeqCst);
                let content = self
                    .responses
                    .get(i)
                    .cloned()
                    .expect("SeqEngine: unexpected extra generate call");
                Ok(EngineResponse {
                    content,
                    prompt_tokens: 0,
                    generated_tokens: 0,
                })
            }
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let engine = SeqEngine {
            responses: Arc::new(vec![
                r#"{"status":"Reflect","tool_calls":[{"name":"system:health","args":{}}]}"#
                    .to_string(),
                r#"{"status":"Idle","tool_calls":[],"message_to_user":"done first turn"}"#
                    .to_string(),
                r#"{"status":"Idle","tool_calls":[],"message_to_user":"alarm handled"}"#.to_string(),
            ]),
            calls: calls.clone(),
        };

        let mut gatekeeper = Gatekeeper::new();
        gatekeeper.register(Arc::new(SystemHealthTool));
        let ephemeral = Arc::new(EphemeralMemory::new("relay_ws".to_string()));
        let dir = tempfile::tempdir().expect("tempdir");
        let vault_root = dir.path();
        let workspace = "relay_ws";
        tokio::fs::create_dir_all(vault_root.join(workspace).join("00_Core"))
            .await
            .expect("mkdir");

        let (watch_tx, watch_rx) = tokio::sync::watch::channel(());
        let _keep_watch = watch_tx;

        let mut orchestrator = Orchestrator::new(
            engine,
            gatekeeper,
            ephemeral,
            vault_root,
            workspace,
            3,
            5,
            0.8,
            4096,
            3,
            6000,
            watch_rx,
            None,
            None,
            None,
        );

        let (action_tx, mut action_rx) = mpsc::channel::<UserAction>(100);
        let long_user =
            "please run a full system health diagnostic because we need relay ordering proof";
        action_tx
            .send(UserAction::Submit(long_user.to_string()))
            .await
            .expect("submit");
        action_tx
            .send(UserAction::SystemInject("Drink water".to_string()))
            .await
            .expect("inject");
        drop(action_tx);

        let mut pending = VecDeque::new();
        let mut saw_inject = false;

        while let Some(action) = action_rx.recv().await {
            match action {
                UserAction::Submit(msg) => {
                    let trimmed = msg.trim().to_string();
                    if !trimmed.is_empty() {
                        pending.push_back(trimmed);
                    }
                }
                UserAction::SystemInject(label) => {
                    assert!(
                        calls.load(AtomicOrdering::SeqCst) >= 2,
                        "tool round must finish (two LLM calls) before alarm is consumed; calls={}",
                        calls.load(AtomicOrdering::SeqCst)
                    );
                    saw_inject = true;
                    let trimmed = label.trim().to_string();
                    let content = format!("{}{}", SYSTEM_ALARM_PREFIX, trimmed);
                    orchestrator.chat_stack.push(Message {
                        role: "user".to_string(),
                        content,
                    });
                    orchestrator.state = AgentState::Chat;
                    orchestrator.step(None).await.expect("alarm step");
                }
                UserAction::CancelCurrentTurn => {}
                UserAction::AgendaAlarmPending { .. } => {}
            }
            while let Some(msg) = pending.pop_front() {
                orchestrator.chat_stack.push(Message {
                    role: "user".to_string(),
                    content: msg,
                });
                orchestrator.state = AgentState::Chat;
                orchestrator.step(None).await.expect("user step");
            }
        }

        assert!(saw_inject, "expected SystemInject to be processed");
        assert_eq!(
            calls.load(AtomicOrdering::SeqCst),
            3,
            "expected three LLM generations: tool, idle, alarm"
        );
        assert!(
            orchestrator.chat_stack.iter().any(|m| {
                m.content.contains("SYSTEM OVERRIDE")
                    && m.content.contains("[SYSTEM OVERRIDE - ALARM TRIGGERED]")
                    && m.content.contains("Drink water")
            }),
            "stack should contain prefixed alarm text"
        );
    }
}
