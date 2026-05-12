use crate::config::AppConfig;
use crate::executive::chat_session::StartedChatSession;
use crate::executive::cli::{Cli, Commands};
use crate::executive::error::{FcpError, Result};
use crate::executive::setup_welder::IgnitionWorkspaceHint;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

async fn log_peripheral_shutdown(session: &mut StartedChatSession, config: &AppConfig) {
    let eris_owned_ollama = session.peripheral_lifecycle.started_ollama();
    tracing::info!("Tearing down peripheral daemons started by this session…");
    let stopped = session.peripheral_lifecycle.shutdown_async().await;
    if stopped.is_empty() {
        tracing::info!(
            "No managed peripheral child processes were stopped (Ollama/Qdrant were already running or not started by Eris)."
        );
    } else {
        tracing::info!(stopped = %stopped.join(", "), "Stopped managed peripheral child processes");
    }
    if config.unload_ollama_models_on_chat_exit && !eris_owned_ollama {
        tracing::info!(
            chat_model = %config.model_name,
            embed_model = %config.embed_model_name,
            "Unloading session models via `ollama stop` (Ollama server was not started by this Eris session)"
        );
        crate::executive::peripherals::unload_ollama_models_cli_best_effort(config).await;
    } else if config.unload_ollama_models_on_chat_exit && eris_owned_ollama {
        tracing::debug!(
            "Skipping `ollama stop`; managed Ollama server for this session was already torn down"
        );
    }
}

pub async fn execute_command(
    cli: Cli,
    config: Arc<AppConfig>,
    cancel_token: CancellationToken,
) -> Result<()> {
    match cli.command {
        Commands::Chat { web: _ } => {
            use crate::executive::chat_session::{ChatViewMode, start_chat_session};
            use crate::ui::terminal::{TuiApp, restore_terminal, setup_terminal};
            use tokio::sync::mpsc;

            let workspace_root = config.active_vault();
            tracing::info!(
                path = %workspace_root.display(),
                workspace = %config.workspace,
                "chat vault root (launch cwd)"
            );

            let seal_path = crate::vault_layout::seal(&workspace_root);
            let ignition_hint: IgnitionWorkspaceHint = if !seal_path.exists() {
                crate::executive::setup_welder::run_welder_before_chat(
                    &cli,
                    config.as_ref(),
                    &workspace_root,
                )
                .await?
            } else {
                IgnitionWorkspaceHint::from_cli(&cli, &workspace_root)
            };

            let view = ChatViewMode::from_cli(&cli);
            let (presentation_tx, presentation_rx) = mpsc::channel(100);

            config.validate_discord_sidecar()?;
            if config.discord.enabled && !config.discord_sidecar_should_run() {
                tracing::warn!(
                    event = "fcp.discord.sidecar_skipped",
                    "Discord is enabled and application/channel are set, but discord.bot_token is missing or empty — chat runs without the Discord sidecar until you add a bot token (Developer Portal → Bot). The public key is for HTTP interactions later, not the gateway."
                );
            }
            let mut discord_mux = if config.discord_sidecar_should_run() {
                let (out_tx, out_rx) = mpsc::channel(config.discord.outbound_queue_capacity.max(1));
                let (typing_tx, typing_rx) = mpsc::channel(8);
                Some((out_tx, out_rx, typing_tx, typing_rx))
            } else {
                None
            };

            match view {
                ChatViewMode::Web => {
                    let session_result = start_chat_session(
                        cli,
                        config.clone(),
                        workspace_root,
                        cancel_token.clone(),
                        presentation_tx,
                        ignition_hint.clone(),
                        discord_mux.as_ref().map(|(_, _, t, _)| t.clone()),
                    )
                    .await;

                    let mut session = session_result?;
                    let web_result = if let Some((dtx, drx, _typing_tx, typing_rx)) =
                        discord_mux.take()
                    {
                        use crate::presentation::multiplex::{
                            PresentationMultiplexTargets, spawn_presentation_multiplex,
                        };
                        use tokio::sync::broadcast;

                        const EVENT_BACKLOG: usize = 512;
                        let (events_tx, _) =
                            broadcast::channel::<crate::presentation::SessionEvent>(EVENT_BACKLOG);
                        let mux_jh = spawn_presentation_multiplex(
                            presentation_rx,
                            PresentationMultiplexTargets {
                                web_broadcast: Some(events_tx.clone()),
                                terminal: None,
                                terminal_omit_system_alarm: false,
                                user_action_tx: session.user_action_tx.clone(),
                                discord_outbound: Some(dtx),
                            },
                        );
                        let disc_jh = tokio::spawn(crate::ui::discord::run_discord_sidecar(
                            config.clone(),
                            session.user_action_tx.clone(),
                            drx,
                            typing_rx,
                            cancel_token.clone(),
                        ));
                        let r = crate::ui::web::run_web_chat_with_broadcast(
                            events_tx,
                            session.user_action_tx.clone(),
                            config.clone(),
                            cancel_token.clone(),
                        )
                        .await;
                        log_peripheral_shutdown(&mut session, config.as_ref()).await;
                        drop(session);
                        let _ = mux_jh.await;
                        match disc_jh.await {
                            Ok(Ok(())) => {}
                            Ok(Err(e)) => {
                                tracing::error!(
                                    event = "fcp.discord.sidecar_failed",
                                    error = %e,
                                    "Discord sidecar exited with error"
                                );
                            }
                            Err(join_err) => {
                                tracing::warn!(
                                    event = "fcp.discord.join_failed",
                                    error = %join_err,
                                    "Discord sidecar task join error"
                                );
                            }
                        }
                        r
                    } else {
                        let r = crate::ui::web::run_web_chat(
                            presentation_rx,
                            session.user_action_tx.clone(),
                            config.clone(),
                            cancel_token.clone(),
                        )
                        .await;
                        log_peripheral_shutdown(&mut session, config.as_ref()).await;
                        r
                    };

                    cancel_token.cancel();
                    web_result
                }
                ChatViewMode::Terminal => {
                    let terminal = setup_terminal()?;
                    let cfg_for_discord = config.clone();
                    let cfg_shutdown = config.clone();

                    let session_result = start_chat_session(
                        cli,
                        config,
                        workspace_root,
                        cancel_token.clone(),
                        presentation_tx,
                        ignition_hint,
                        discord_mux.as_ref().map(|(_, _, t, _)| t.clone()),
                    )
                    .await;

                    let mut session = match session_result {
                        Ok(s) => s,
                        Err(e) => {
                            let _ = restore_terminal();
                            return Err(e);
                        }
                    };

                    let result = if let Some((dtx, drx, _typing_tx, typing_rx)) = discord_mux.take()
                    {
                        use crate::presentation::multiplex::{
                            PresentationMultiplexTargets, spawn_presentation_multiplex,
                        };

                        let (tui_tx, tui_rx) = mpsc::channel(256);
                        let mux_jh = spawn_presentation_multiplex(
                            presentation_rx,
                            PresentationMultiplexTargets {
                                web_broadcast: None,
                                terminal: Some(tui_tx),
                                terminal_omit_system_alarm: true,
                                user_action_tx: session.user_action_tx.clone(),
                                discord_outbound: Some(dtx),
                            },
                        );
                        let disc_jh = tokio::spawn(crate::ui::discord::run_discord_sidecar(
                            cfg_for_discord,
                            session.user_action_tx.clone(),
                            drx,
                            typing_rx,
                            cancel_token.clone(),
                        ));
                        let mut app = TuiApp::new(tui_rx, session.user_action_tx.clone());
                        let token_metrics_rx = session.token_metrics_rx.clone();
                        let r = app.run(terminal, Some(token_metrics_rx)).await;
                        log_peripheral_shutdown(&mut session, cfg_shutdown.as_ref()).await;
                        drop(session);
                        let _ = mux_jh.await;
                        match disc_jh.await {
                            Ok(Ok(())) => {}
                            Ok(Err(e)) => {
                                tracing::error!(
                                    event = "fcp.discord.sidecar_failed",
                                    error = %e,
                                    "Discord sidecar exited with error"
                                );
                            }
                            Err(join_err) => {
                                tracing::warn!(
                                    event = "fcp.discord.join_failed",
                                    error = %join_err,
                                    "Discord sidecar task join error"
                                );
                            }
                        }
                        r
                    } else {
                        let mut app = TuiApp::new(presentation_rx, session.user_action_tx.clone());
                        let token_metrics_rx = session.token_metrics_rx.clone();
                        let r = app.run(terminal, Some(token_metrics_rx)).await;
                        log_peripheral_shutdown(&mut session, cfg_shutdown.as_ref()).await;
                        r
                    };

                    cancel_token.cancel();
                    restore_terminal()?;
                    result
                }
            }
        }
        Commands::Benchmark {
            suite,
            format,
            compare,
            diff,
            diff_files,
            diff_vaults,
            list,
            trend,
            isolation,
            output,
            no_dry_run,
            i_understand_risks,
            no_cleanup,
        } => {
            // Safety check: require explicit acknowledgment to disable dry-run
            if no_dry_run && !i_understand_risks {
                return Err(FcpError::Config(
                    "Disabling dry-run mode requires --i-understand-risks flag. \
                     This may allow external side effects.".to_string()
                ));
            }

            // Parse isolation mode
            let isolation_mode = isolation.parse::<crate::benchmark::IsolationMode>()
                .map_err(|e| FcpError::Config(format!("Invalid isolation mode: {}", e)))?;

            if diff.as_ref().is_some() && diff_files.as_ref().is_some() {
                return Err(FcpError::Config(
                    "Use either --diff (same vault, run IDs) or --diff-files (two JSON paths), not both."
                        .into(),
                ));
            }

            // Latest report from each vault directory (siblings under cwd).
            if let Some(names) = diff_vaults {
                if names.len() != 2 {
                    return Err(FcpError::Config(
                        "--diff-vaults requires exactly two vault directory names".into(),
                    ));
                }
                let cwd = std::env::current_dir().map_err(|e| {
                    FcpError::Io(std::io::Error::other(format!(
                        "Could not read working directory: {e}"
                    )))
                })?;
                let baseline_root = cwd.join(names[0].trim());
                let current_root = cwd.join(names[1].trim());
                for (label, p) in [("baseline", &baseline_root), ("current", &current_root)] {
                    if !p.is_dir() {
                        return Err(FcpError::Config(format!(
                            "--diff-vaults {label}: not a directory: {}",
                            p.display()
                        )));
                    }
                }

                let baseline_storage =
                    crate::benchmark::BenchmarkStorage::for_vault(&baseline_root)?;
                let current_storage =
                    crate::benchmark::BenchmarkStorage::for_vault(&current_root)?;

                let baseline = baseline_storage.load_latest().map_err(|e| {
                    FcpError::Config(format!(
                        "Could not load latest benchmark for baseline vault {} (run a benchmark there first): {}",
                        baseline_root.display(),
                        e
                    ))
                })?;
                let current = current_storage.load_latest().map_err(|e| {
                    FcpError::Config(format!(
                        "Could not load latest benchmark for current vault {} (run a benchmark there first): {}",
                        current_root.display(),
                        e
                    ))
                })?;

                let comparison =
                    crate::benchmark::reporter::ReportGenerator::comparison(&baseline, &current);
                println!("{}", comparison);
                println!(
                    "\nCompared latest runs:\n  baseline ← {}\n  current  ← {}",
                    baseline_root.display(),
                    current_root.display()
                );
                return Ok(());
            }

            // Compare arbitrary report files (cross-vault / cross-model)
            if let Some(paths) = diff_files {
                if paths.len() != 2 {
                    return Err(FcpError::Config(
                        "--diff-files requires exactly two JSON paths".into(),
                    ));
                }
                let baseline_path = &paths[0];
                let current_path = &paths[1];
                let baseline = crate::benchmark::storage::load_report_from_file(baseline_path)?;
                let current = crate::benchmark::storage::load_report_from_file(current_path)?;
                let comparison =
                    crate::benchmark::reporter::ReportGenerator::comparison(&baseline, &current);
                println!("{}", comparison);
                return Ok(());
            }

            // Handle list mode
            if list {
                let storage = crate::benchmark::BenchmarkStorage::for_vault(&config.active_vault())?;
                let reports = storage.list_reports()?;

                println!("\nAvailable benchmark runs:");
                println!("{}", "─".repeat(80));
                println!("{:<25} {:<20} {:<12} {:<8} {}",
                    "Timestamp", "Model", "Suite", "Quality", "Run ID");
                println!("{}", "─".repeat(80));

                for info in reports {
                    println!("{}", info.format_for_list());
                }

                println!("{}", "─".repeat(80));
                println!("\nSame vault:     eris benchmark --diff '<run-id-1>..<run-id-2>'");
                println!("Sibling vaults: eris benchmark --diff-vaults <dir-a> <dir-b>   (from parent folder)");
                println!("Cross-file:     eris benchmark --diff-files BASELINE.json CURRENT.json");
                return Ok(());
            }

            // Handle trend mode
            if let Some(count) = trend {
                let storage = crate::benchmark::BenchmarkStorage::for_vault(&config.active_vault())?;
                let reports = storage.get_trend_reports(count)?;

                if reports.len() < 2 {
                    println!("Need at least 2 runs for trend analysis (found {})", reports.len());
                    return Ok(());
                }

                let trend_output = crate::benchmark::reporter::ReportGenerator::generate_trend_report(&reports);
                println!("{}", trend_output);

                if let Some(path) = output {
                    let md = crate::benchmark::reporter::ReportGenerator::generate_trend_report(&reports);
                    tokio::fs::write(&path, md).await?;
                    println!("Trend report saved to: {}", path.display());
                }

                return Ok(());
            }

            // Handle diff mode
            if let Some(diff_arg) = diff {
                let storage = crate::benchmark::BenchmarkStorage::for_vault(&config.active_vault())?;
                let (baseline_id, current_id) = crate::benchmark::storage::parse_diff_argument(&diff_arg)?;

                let baseline = storage.load_report(&baseline_id)?;
                let current = storage.load_report(&current_id)?;

                let comparison = crate::benchmark::reporter::ReportGenerator::comparison(&baseline, &current);
                println!("{}", comparison);
                return Ok(());
            }

            // Normal benchmark run
            println!("Starting benchmark: suite={}, format={}, isolation={}",
                suite, format, isolation);

            // Run the benchmark
            let report = crate::benchmark::run_benchmark(
                &config,
                &suite,
                &format,
                compare,
                output.clone(),
                isolation_mode,
            ).await?;

            // Save report to storage
            let storage = crate::benchmark::BenchmarkStorage::for_vault(&config.active_vault())?;
            storage.save_report(&report)?;

            // Handle comparison with previous run if requested
            if compare {
                match storage.load_latest() {
                    Ok(previous) => {
                        if previous.run_id != report.run_id {
                            let comparison = crate::benchmark::reporter::ReportGenerator::comparison(
                                &previous, &report
                            );
                            println!("\n{}", comparison);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Could not load previous run for comparison: {}", e);
                    }
                }
            }

            // Handle no_cleanup flag
            if no_cleanup {
                tracing::warn!("--no-cleanup specified: benchmark artifacts retained for debugging");
            }

            Ok(())
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

        let cmd = Commands::Run {
            prompt: "test".to_string(),
        };
        let result = execute_command(test_cli(cmd), test_config(), cancel_token).await;
        assert!(result.is_ok());
    }

    /// Submit queues work that runs `system:health` (Reflect), then `SystemInject` is already on
    /// `action_rx`. The first `step` must fully finish (tool + follow-up generation) before the
    /// relay pulls the alarm—FIFO on the single action channel.
    #[tokio::test]
    async fn relay_submit_then_system_inject_orders_after_tool() {
        use std::collections::VecDeque;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

        use async_trait::async_trait;
        use tokio::sync::mpsc;

        use crate::engine::{EngineResponse, LlmEngine, Message};
        use crate::memory::ephemeral::EphemeralMemory;
        use crate::orchestrator::core::Orchestrator;
        use crate::orchestrator::state::AgentState;
        use crate::presentation::{SYSTEM_ALARM_PREFIX, UserAction};
        use crate::tools::gatekeeper::Gatekeeper;
        use crate::tools::system::SystemHealthTool;

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
                    generation_ms: 0,
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
                r#"{"status":"Idle","tool_calls":[],"message_to_user":"alarm handled"}"#
                    .to_string(),
            ]),
            calls: calls.clone(),
        };

        let mut gatekeeper = Gatekeeper::new();
        gatekeeper.register(Arc::new(SystemHealthTool {
            config: test_config(),
        }));
        let ephemeral = Arc::new(EphemeralMemory::new("relay_ws".to_string()));
        let dir = tempfile::tempdir().expect("tempdir");
        let vault_root = dir.path();
        let workspace = "relay_ws";
        tokio::fs::create_dir_all(vault_root.join(workspace).join("00_Invariants"))
            .await
            .expect("mkdir");

        let (watch_tx, watch_rx) = tokio::sync::watch::channel(());
        let _keep_watch = watch_tx;

        let (_id_tx, id_rx) = tokio::sync::watch::channel(std::sync::Arc::from(
            "relay test identity for orchestrator",
        ));

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
            false,
            0,
            watch_rx,
            None,
            None,
            None,
            crate::orchestrator::context::ContextViewSettings::default(),
            Arc::new(crate::config::AppConfig::default()),
            id_rx,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
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
                UserAction::SubmitIngress(ing) => {
                    let content = ing.for_model.unwrap_or_else(|| ing.display.clone());
                    let trimmed = content.trim().to_string();
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
                UserAction::AgendaSelfPrompt { .. } => {}
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
