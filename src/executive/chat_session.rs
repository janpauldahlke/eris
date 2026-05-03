//! Chat runtime: vault/tool/orchestrator bootstrap and the single [`UserAction`] consumer task.
//! Terminal (or future web) owns only transport: [`crate::presentation`] channels and view-specific run.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::config::AppConfig;
use crate::engine::ollama::OllamaClient;
use crate::engine::token_metrics::LlmTokenSnapshot;
use crate::executive::cli::Cli;
use crate::executive::error::{FcpError, Result};
use crate::executive::ignition::IgnitionOptions;
use crate::executive::peripherals::PeripheralLifecycle;
use crate::executive::setup_welder::IgnitionWorkspaceHint;
use crate::memory::ephemeral::EphemeralMemory;
use crate::orchestrator::core::Orchestrator;
use crate::presentation::{
    InputSource, SessionEvent, UserAction, UserIngress, SYSTEM_ALARM_PREFIX,
};
use crate::ui::discord::DiscordTypingCtl;
use crate::tools::Gatekeeper;
use ollama_rs::Ollama;

/// Which presentation surface runs for this process (one only; never both).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatViewMode {
    Terminal,
    /// Axum + SSE browser UI (`eris chat --web`).
    Web,
}

impl ChatViewMode {
    /// Resolve from CLI until config-driven flags exist.
    pub fn from_cli(cli: &Cli) -> Self {
        match &cli.command {
            crate::executive::cli::Commands::Chat { web: true } => ChatViewMode::Web,
            _ => ChatViewMode::Terminal,
        }
    }
}

fn try_send_discord_typing(tx: &Option<mpsc::Sender<DiscordTypingCtl>>, cmd: DiscordTypingCtl) {
    if let Some(t) = tx {
        if t.try_send(cmd).is_err() {
            tracing::debug!(
                event = "fcp.discord.typing_ctl_dropped",
                cmd = ?cmd,
                "Discord typing control not delivered (channel full or sidecar gone)"
            );
        }
    }
}

async fn enqueue_user_ingress(
    pending_inputs: &mut VecDeque<UserIngress>,
    presentation_tx_err: &mpsc::Sender<SessionEvent>,
    mut ing: UserIngress,
) {
    ing.display = ing.display.trim().to_string();
    if ing.display.is_empty() {
        return;
    }
    if pending_inputs.len() >= 3 {
        let _ = pending_inputs.pop_front();
        let _ = presentation_tx_err
            .send(SessionEvent::SystemError(
                "[ui] Queue full; dropped oldest queued input".into(),
            ))
            .await;
    }
    pending_inputs.push_back(ing);
    if pending_inputs.len() > 1 {
        let _ = presentation_tx_err
            .send(SessionEvent::SystemError(format!(
                "[ui] Processing older request ({} newer queued)",
                pending_inputs.len() - 1
            )))
            .await;
    }
}

/// Handle returned after chat core is running: wire the active view to these ends.
pub struct StartedChatSession {
    pub user_action_tx: mpsc::Sender<UserAction>,
    pub token_metrics_rx: watch::Receiver<LlmTokenSnapshot>,
    pub peripheral_lifecycle: PeripheralLifecycle,
}

/// Bootstrap engine, tools, orchestrator, and background tasks. Caller keeps `presentation_rx` for the view.
///
/// Send startup lines on `presentation_tx` before heavy work so the terminal can show progress.
///
/// When `discord_typing_tx` is set, [`DiscordTypingCtl`] is sent for Discord-originated turns only;
/// it never emits [`SessionEvent`]s and does not affect web or TUI.
pub async fn start_chat_session(
    cli: Cli,
    mut config: Arc<AppConfig>,
    workspace_root: PathBuf,
    cancel_token: CancellationToken,
    presentation_tx: mpsc::Sender<SessionEvent>,
    ignition_workspace: IgnitionWorkspaceHint,
    discord_typing_tx: Option<mpsc::Sender<DiscordTypingCtl>>,
) -> Result<StartedChatSession> {
    let seal_path = crate::vault_layout::seal(&workspace_root);
    if !seal_path.exists() {
        crate::executive::ignition::run_ignition_sequence(
            &workspace_root,
            IgnitionOptions {
                workspace: ignition_workspace.workspace,
            },
        )
        .await?;
        config = Arc::new(AppConfig::load(cli.clone())?);
    }
    crate::executive::identity_md::sync_identity_user_line(&workspace_root, &config.user_name)
        .await?;

    let default_identity = workspace_root.join("00_Invariants/Identity.md");
    let mut identity_path = default_identity.clone();
    let mut upload_dirs: Vec<PathBuf> = Vec::new();
    let mut extra_watched_files: Vec<PathBuf> = Vec::new();

    for rel in &config.vault_watch.paths {
        let p = workspace_root.join(rel);
        let norm = rel.replace('\\', "/");
        let norm_trim = norm.trim_end_matches('/');
        if norm_trim.ends_with("Identity.md") {
            identity_path = p;
        } else if norm_trim == "99_USER_UPLOADED" || norm_trim.ends_with("/99_USER_UPLOADED") {
            upload_dirs.push(p);
        } else {
            extra_watched_files.push(p);
        }
    }

    let mut watched_files = vec![identity_path.clone()];
    watched_files.extend(extra_watched_files);
    watched_files.sort();
    watched_files.dedup();
    let initial_identity = crate::executive::vault_identity::read_identity_markdown_strict(
        &config.workspace,
        &identity_path,
    )
    .await?;
    tracing::info!(
        target: "fcp.vault_watch",
        path = %identity_path.display(),
        len = initial_identity.len(),
        phase = "initial_load",
        "identity snapshot loaded for chat"
    );
    let (identity_tx, identity_rx) = tokio::sync::watch::channel(initial_identity);

    if config.vault_watch.enabled {
        let debounce = std::time::Duration::from_millis(config.vault_watch.debounce_ms.max(1));
        crate::util::fs_watch::spawn_vault_identity_watch(
            cancel_token.child_token(),
            debounce,
            identity_path.clone(),
            watched_files,
            upload_dirs,
            identity_tx,
        );
    } else {
        drop(identity_tx);
    }

    let _ = presentation_tx
        .send(SessionEvent::SystemError(
            "[startup] Checking peripheral daemons (Ollama, Qdrant)...".into(),
        ))
        .await;

    let peripheral_lifecycle =
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
    let _ = presentation_tx
        .send(SessionEvent::SystemError(format!(
            "[startup] Peripheral readiness: ollama={ollama_status}, qdrant={qdrant_status}"
        )))
        .await;

    let parsed_url = url::Url::parse(&config.ollama_host)
        .map_err(|e| FcpError::Config(format!("Invalid ollama_host URL: {}", e)))?;
    let host = format!(
        "{}://{}",
        parsed_url.scheme(),
        parsed_url.host_str().unwrap_or("localhost")
    );
    let port = parsed_url.port().unwrap_or(11434);

    let client = Ollama::new(host, port);
    let (token_metrics_tx, token_metrics_rx) = crate::engine::token_metrics::channel();
    let engine = OllamaClient::with_token_metrics(client.clone(), config.clone(), token_metrics_tx);
    let ollama_arc = Arc::new(client);
    let ephemeral = Arc::new(EphemeralMemory::new(config.workspace.clone()));
    let connect_attempts = config.semantic_brain_connect_attempts;
    let connect_retry_ms = config.semantic_brain_connect_retry_delay_ms;
    let semantic_arc: Option<Arc<crate::memory::semantic::SemanticBrain>> =
        match crate::memory::semantic::SemanticBrain::new_with_connect_retries(
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

                match semantic.ingest_vault_v2(&workspace_root).await {
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

    let api_http = Arc::new(crate::util::ApiHttpClient::new(config.clone())?);

    let mut gatekeeper = Gatekeeper::new();
    let (alarm_reschedule_tx, alarm_reschedule_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let read_limit = (config.num_ctx as f32 * config.vault_read_ratio) as usize;
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
    gatekeeper.register(Arc::new(crate::tools::vault::VaultSearchTool {
        workspace_root: workspace_root.clone(),
        max_files: config.vault_search_max_files,
        max_snippets_per_file: config.vault_search_max_snippets_per_file,
        snippet_radius_lines: config.vault_search_snippet_radius_lines,
        max_total_chars: config.vault_search_max_total_chars,
        max_file_bytes: config.vault_search_max_file_bytes,
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
    if config.web_fetch_deprecated {
        tracing::info!("web:fetch deprecated by config — not registered");
    } else {
        gatekeeper.register(Arc::new(crate::tools::web::WebFetchTool::new(
            config.web_fetch_timeout_secs,
            effective_web_fetch_max_bytes,
            web_chunk_chars,
            web_preview_chars,
            config.ephemeral_ttl_session_secs,
            config.web_fetch_user_agent.clone(),
            config.web_fetch_default_referer.clone(),
            ephemeral.clone(),
            semantic_arc.clone(),
        )));
        gatekeeper.register(Arc::new(crate::tools::news::NewsTodayTool::new(
            config.web_fetch_timeout_secs,
            effective_web_fetch_max_bytes,
            web_chunk_chars,
            web_preview_chars,
            config.ephemeral_ttl_session_secs,
            config.web_fetch_user_agent.clone(),
            config.web_fetch_default_referer.clone(),
            ephemeral.clone(),
            semantic_arc.clone(),
            crate::tools::news::NewsTodayConfigSnapshot {
                site_base: config.news_today_site_base.clone(),
                default_homepage: config.news_today_default_homepage.clone(),
                max_headlines_default: config.news_today_max_headlines_default,
                deep_fetch_max_default: config.news_today_deep_fetch_max_default,
                allowed_hosts: config.news_today_allowed_hosts.clone(),
            },
        )));
    }
    gatekeeper.register(Arc::new(crate::tools::web::WebArtifactQueryTool {
        ephemeral: ephemeral.clone(),
        semantic: semantic_arc.clone(),
        max_snippet_chars: (web_chunk_chars / 3).clamp(300, 900),
        max_total_chars: (web_chunk_chars / 2).clamp(1000, 2500),
    }));
    gatekeeper.register(Arc::new(crate::tools::system::SystemHealthTool {
        config: config.clone(),
    }));

    gatekeeper.register(Arc::new(crate::tools::clock::ClockNowTool));
    gatekeeper.register(Arc::new(crate::tools::clock::ClockTimerTool {
        workspace_root: workspace_root.clone(),
        reschedule_tx: alarm_reschedule_tx.clone(),
    }));
    gatekeeper.register(Arc::new(crate::tools::clock::ClockWallAlarmTool {
        workspace_root: workspace_root.clone(),
        reschedule_tx: alarm_reschedule_tx,
    }));

    gatekeeper.register(Arc::new(crate::tools::weather::WeatherCurrentTool {
        api: api_http.clone(),
    }));
    gatekeeper.register(Arc::new(crate::tools::weather::WeatherForecastTool {
        api: api_http.clone(),
    }));
    gatekeeper.register(Arc::new(crate::tools::db_rest::DbFindConnectionsTool {
        api: api_http.clone(),
    }));
    gatekeeper.register(Arc::new(crate::tools::wiki::WikiSummaryTool {
        api: api_http,
    }));

    if let Some(auth) =
        crate::util::google_workspace::workspace_auth(&config.google).await?
    {
        let gmail = Arc::new(crate::util::GmailClient::from_auth(auth.clone())?);
        gatekeeper.register(Arc::new(crate::tools::mail::MailCheckTool {
            client: gmail.clone(),
        }));
        gatekeeper.register(Arc::new(crate::tools::mail::MailReadTool {
            client: gmail.clone(),
        }));
        gatekeeper.register(Arc::new(crate::tools::mail::MailDigestTool {
            client: gmail.clone(),
        }));
        gatekeeper.register(Arc::new(crate::tools::mail::MailDeleteTool {
            client: gmail.clone(),
        }));
        gatekeeper.register(Arc::new(crate::tools::mail::MailMoveTool {
            client: gmail.clone(),
        }));
        gatekeeper.register(Arc::new(crate::tools::mail::MailWriteTool {
            client: gmail,
        }));

        let calendar = Arc::new(crate::util::CalendarClient::from_auth(auth)?);
        gatekeeper.register(Arc::new(crate::tools::calendar::CalendarListTool {
            client: calendar.clone(),
        }));
        gatekeeper.register(Arc::new(crate::tools::calendar::CalendarGetTool {
            client: calendar.clone(),
        }));
        gatekeeper.register(Arc::new(crate::tools::calendar::CalendarCreateTool {
            client: calendar.clone(),
        }));
        gatekeeper.register(Arc::new(crate::tools::calendar::CalendarUpdateTool {
            client: calendar.clone(),
        }));
        gatekeeper.register(Arc::new(crate::tools::calendar::CalendarDeleteTool {
            client: calendar,
        }));
    }

    let max_content_chars = config.num_ctx * 3;
    gatekeeper.register(Arc::new(crate::tools::memory::MemoryStageTool {
        ephemeral: ephemeral.clone(),
        config: config.clone(),
        max_content_chars,
    }));
    gatekeeper.register(Arc::new(crate::tools::memory::MemoryStagedListTool {
        ephemeral: ephemeral.clone(),
    }));

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
            default_top_k: config.memory_query_default_top_k,
            top_k_max: config.memory_query_top_k_max,
            default_max_total_chars: config.memory_query_default_max_total_chars,
            min_max_total_chars: config.memory_query_min_max_total_chars,
            qdrant_oversample_cap: config.memory_query_oversample_cap,
            qdrant_oversample_multiplier: config.memory_query_oversample_multiplier,
            qdrant_oversample_min: config.memory_query_oversample_min,
        }));
    }

    let descriptor_registry = {
        let registry = crate::tools::ToolDescriptorRegistry::load_embedded()?;
        registry.assert_covers_registered_tools(&gatekeeper.registered_tool_names())?;
        tracing::info!(
            descriptor_count = registry.len(),
            "Embedded tool descriptor registry loaded"
        );
        Some(Arc::new(registry))
    };

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
            tracing::warn!(
                error = %e,
                "ToolRouter offline — all requests will include tool schemas."
            );
            None
        }
    };

    let (interrupt_tx, interrupt_rx) = tokio::sync::watch::channel(());
    let last_input_time = Arc::new(std::sync::atomic::AtomicU64::new(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    ));

    if config.idle_heartbeat_enabled {
        crate::orchestrator::heartbeat::spawn_heartbeat_monitor(
            last_input_time.clone(),
            config.idle_timeout_secs,
            interrupt_tx.clone(),
            cancel_token.clone(),
        );
        tracing::info!(
            idle_timeout_secs = config.idle_timeout_secs,
            "Idle heartbeat monitor spawned"
        );
    } else {
        tracing::info!(
            "Idle heartbeat disabled (idle_heartbeat_enabled = false); Esc cancel still active"
        );
    }

    crate::orchestrator::alarms::spawn_alarm_scheduler(
        workspace_root.clone(),
        presentation_tx.clone(),
        alarm_reschedule_rx,
        cancel_token.clone(),
    );

    let startup_wp = workspace_root.clone();
    let startup_presentation = presentation_tx.clone();
    tokio::spawn(async move {
        if let Some(msg) =
            crate::orchestrator::alarms::startup_overdue_agenda_hint(&startup_wp).await
        {
            let _ = startup_presentation
                .send(SessionEvent::SystemError(msg))
                .await;
        }
    });

    let promotion_suppressed_during_step = Arc::new(std::sync::atomic::AtomicBool::new(false));
    crate::memory::ephemeral::spawn_snapshot_daemon(
        ephemeral.clone(),
        workspace_root.clone(),
        semantic_arc,
        config.snapshot_interval_secs,
        cancel_token.clone(),
        config.clone(),
        promotion_suppressed_during_step.clone(),
    );

    let context_view_hints = gatekeeper.merge_context_view_hints(&config.optimize_context_tool_overrides);
    let context_view = crate::orchestrator::context::ContextViewSettings {
        enabled: config.optimize_context,
        default_snippet_chars: config.optimize_context_max_tool_snippet_chars,
        assistant_compact: config.optimize_context_assistant_compact,
        full_tool_schemas_in_llm_view: config.optimize_context_full_tool_schemas,
        omit_resolved_tool_recovery: config.optimize_context_omit_resolved_tool_recovery,
        assistant_non_json_placeholder: config.optimize_context_assistant_non_json_placeholder,
        hints: Arc::new(context_view_hints),
    };

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
        config.slim_tool_prompt,
        config.tool_map_offer_cap,
        interrupt_rx,
        Some(presentation_tx.clone()),
        tool_router,
        descriptor_registry,
        context_view,
        config.clone(),
        identity_rx,
        promotion_suppressed_during_step,
    );

    tracing::info!(
        model = %config.model_name,
        num_ctx = config.num_ctx,
        max_tool_rounds = config.max_tool_rounds,
        max_recovery = config.max_recovery_attempts,
        "Orchestrator initialized"
    );

    let submit_source_default = match ChatViewMode::from_cli(&cli) {
        ChatViewMode::Web => InputSource::Web,
        ChatViewMode::Terminal => InputSource::Cli,
    };

    let (user_action_tx, mut action_rx) = mpsc::channel::<UserAction>(100);
    let presentation_tx_err = presentation_tx.clone();
    let cancel_token_loop = cancel_token.clone();
    let interrupt_tx_user = interrupt_tx.clone();
    let discord_typing_loop = discord_typing_tx.clone();
    tokio::spawn(async move {
        let mut pending_inputs: VecDeque<UserIngress> = VecDeque::new();
        loop {
            tokio::select! {
                Some(action) = action_rx.recv() => {
                    match action {
                        UserAction::CancelCurrentTurn => {
                            tracing::info!("User requested cancel current turn");
                            let _ = interrupt_tx_user.send(());
                            let _ = presentation_tx_err.send(SessionEvent::SystemError("[ui] Cancel requested".into())).await;
                        }
                        UserAction::SystemInject(label) => {
                            let trimmed = label.trim().to_string();
                            if trimmed.is_empty() {
                                continue;
                            }
                            let content = format!("{}{}", SYSTEM_ALARM_PREFIX, trimmed);
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
                                let _ = presentation_tx_err
                                    .send(SessionEvent::SystemError(err_msg))
                                    .await;
                                break;
                            }
                            orchestrator.broadcast_state().await;
                        }
                        UserAction::AgendaAlarmPending {
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
                                SYSTEM_ALARM_PREFIX,
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
                                let _ = presentation_tx_err
                                    .send(SessionEvent::SystemError(err_msg))
                                    .await;
                                break;
                            }
                            orchestrator.broadcast_state().await;
                        }
                        UserAction::Submit(msg) => {
                            let display = msg.trim().to_string();
                            if display.is_empty() {
                                continue;
                            }
                            enqueue_user_ingress(
                                &mut pending_inputs,
                                &presentation_tx_err,
                                UserIngress {
                                    source: submit_source_default,
                                    display,
                                    for_model: None,
                                },
                            )
                            .await;
                        }
                        UserAction::SubmitIngress(ing) => {
                            enqueue_user_ingress(&mut pending_inputs, &presentation_tx_err, ing).await;
                        }
                    }
                }
                _ = cancel_token_loop.cancelled() => {
                    tracing::info!("Orchestrator loop received cancellation signal");
                    break;
                }
            }

            while let Some(ing) = pending_inputs.pop_front() {
                orchestrator.queued_inputs = pending_inputs.len();
                orchestrator.broadcast_state().await;
                let for_model = ing
                    .for_model
                    .unwrap_or_else(|| ing.display.clone());
                tracing::info!(
                    msg_len = for_model.len(),
                    queued = pending_inputs.len(),
                    source = ?ing.source,
                    "User input received"
                );
                if presentation_tx_err
                    .send(SessionEvent::UserTranscriptLine {
                        source: ing.source,
                        body: ing.display.clone(),
                    })
                    .await
                    .is_err()
                {
                    tracing::warn!(
                        event = "fcp.chat.user_transcript_dropped",
                        "Presentation channel closed; user transcript line not delivered to views"
                    );
                }
                last_input_time.store(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                    std::sync::atomic::Ordering::Relaxed,
                );
                orchestrator.chat_stack.push(crate::engine::Message {
                    role: "user".to_string(),
                    content: for_model,
                });
                orchestrator.state = crate::orchestrator::state::AgentState::Chat;
                let pulse_discord = ing.source == InputSource::Discord;
                if pulse_discord {
                    try_send_discord_typing(&discord_typing_loop, DiscordTypingCtl::StartPulse);
                }
                let step_result = orchestrator.step(None).await;
                if pulse_discord {
                    try_send_discord_typing(&discord_typing_loop, DiscordTypingCtl::StopPulse);
                }
                if let Err(e) = step_result {
                    if matches!(e, FcpError::Interrupted) {
                        tracing::info!("Orchestrator interrupted by heartbeat, continuing loop");
                        continue;
                    }
                    let err_msg = format!("[FATAL ERROR] Orchestrator halted: {}", e);
                    tracing::error!(error = %e, "Orchestrator fatal error");
                    let _ = presentation_tx_err
                        .send(SessionEvent::SystemError(err_msg))
                        .await;
                    break;
                }
                orchestrator.queued_inputs = pending_inputs.len();
                orchestrator.broadcast_state().await;
            }
        }
    });

    Ok(StartedChatSession {
        user_action_tx,
        token_metrics_rx,
        peripheral_lifecycle,
    })
}
