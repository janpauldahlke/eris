//! Chat runtime: vault/tool/orchestrator bootstrap and the single [`UserAction`] consumer task.
//! Terminal (or future web) owns only transport: [`crate::presentation`] channels and view-specific run.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::config::{AppConfig, LlmBackend};
use crate::engine::ollama::OllamaClient;
use crate::engine::token_metrics::LlmTokenSnapshot;
use crate::engine::{AnyEngine, LlamaCppClient};
use crate::executive::cli::Cli;
use crate::executive::error::{FcpError, Result};
use crate::executive::ignition::IgnitionOptions;
use crate::executive::peripherals::PeripheralLifecycle;
use crate::executive::setup_welder::IgnitionWorkspaceHint;
use crate::memory::ephemeral::EphemeralMemory;
use crate::orchestrator::core::Orchestrator;
use crate::presentation::{
    InputSource, SYSTEM_ALARM_PREFIX, SYSTEM_SELF_REMINDER_PREFIX, SessionEvent, UserAction,
    UserIngress,
};
use crate::tools::Gatekeeper;
use crate::ui::discord::DiscordTypingCtl;
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

/// LLM-facing user content: optional vision path hint when an image attachment is present.
fn build_user_for_model(ing: &UserIngress) -> String {
    if let Some(ref img) = ing.image {
        let prompt = ing
            .for_model
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(ing.display.as_str());
        format!(
            "{prompt}\n\n[Attached image at vault path: {path}]\nBefore answering any question about this image, call vision:see with relative_path \"{path}\" and a prompt that matches what the user asked.",
            path = img.relative_path,
        )
    } else {
        ing.for_model
            .clone()
            .unwrap_or_else(|| ing.display.clone())
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
        if ing.image.is_some() {
            ing.display = "(image attachment)".to_string();
        } else if ing.audio.is_some() {
            // STT fills display before the orchestrator turn.
        } else {
            return;
        }
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

    let backend_label = &config.llm_backend;
    let _ = presentation_tx
        .send(SessionEvent::SystemError(format!(
            "[startup] Checking peripheral daemons ({backend_label}, Qdrant)..."
        )))
        .await;

    let peripheral_lifecycle =
        crate::executive::peripherals::ensure_peripherals_for_chat(&config).await?;

    let llm_status = match config.llm_backend {
        crate::config::LlmBackend::Ollama => {
            if peripheral_lifecycle.started_ollama() {
                "ollama=started by eris"
            } else {
                "ollama=already running"
            }
            .to_string()
        }
        crate::config::LlmBackend::LlamaCpp => {
            let chat = if peripheral_lifecycle.started_llama_chat() {
                "started by eris"
            } else {
                "external"
            };
            let embed = if peripheral_lifecycle.started_llama_embed() {
                "started by eris"
            } else {
                "external"
            };
            format!("llama-chat={chat}, llama-embed={embed}")
        }
    };
    let qdrant_status = if peripheral_lifecycle.started_qdrant() {
        "started by eris"
    } else {
        "already running"
    };
    let _ = presentation_tx
        .send(SessionEvent::SystemError(format!(
            "[startup] Peripheral readiness: {llm_status}, qdrant={qdrant_status}"
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
    let mut engine: AnyEngine = match config.llm_backend {
        LlmBackend::Ollama => {
            let ollama_engine =
                OllamaClient::with_token_metrics(client.clone(), config.clone(), token_metrics_tx);
            AnyEngine::Ollama(ollama_engine)
        }
        LlmBackend::LlamaCpp => {
            let llamacpp_engine = LlamaCppClient::new(config.clone())?
                .with_token_metrics(token_metrics_tx);
            AnyEngine::LlamaCpp(llamacpp_engine)
        }
    };
    let ollama_arc = Arc::new(client);

    let embed_provider: Arc<dyn crate::engine::EmbeddingProvider> = match config.llm_backend {
        crate::config::LlmBackend::Ollama => Arc::new(
            crate::engine::embedding::OllamaEmbedding::new(
                ollama_arc.clone(),
                config.embed_model_name.clone(),
            ),
        ),
        crate::config::LlmBackend::LlamaCpp => {
            let lc = config.validate_llamacpp_config()?;
            Arc::new(crate::engine::embedding::LlamaCppEmbedding::new(
                &lc.embed_server_url,
                config.generation_timeout_secs,
            )?)
        }
    };

    crate::memory::semantic::validate_embedding_provider_vs_qdrant(
        config.as_ref(),
        embed_provider.dimensions(),
    )
    .await?;

    let ephemeral = Arc::new(EphemeralMemory::new(config.workspace.clone()));
    let connect_attempts = config.semantic_brain_connect_attempts;
    let connect_retry_ms = config.semantic_brain_connect_retry_delay_ms;
    let semantic_arc: Option<Arc<crate::memory::semantic::SemanticBrain>> =
        match crate::memory::semantic::SemanticBrain::new_with_connect_retries(
            config.clone(),
            embed_provider.clone(),
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

    config.validate_vision_ready()?;
    config.validate_audio_ready()?;

    let api_http = Arc::new(crate::util::ApiHttpClient::new(config.clone())?);

    let mut gatekeeper = Gatekeeper::new();
    let (alarm_reschedule_tx, alarm_reschedule_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let read_limit = (config.num_ctx as f32 * config.vault_read_ratio) as usize;
    let web_fetch_chunk_chars = config.resolved_web_fetch_chunk_chars();
    let effective_web_fetch_max_bytes = config
        .web_fetch_max_bytes
        .min(web_fetch_chunk_chars.saturating_mul(6))
        .max(web_fetch_chunk_chars);

    if config.moltbook.enabled {
        match crate::tools::moltbook::MoltbookClient::unauthenticated(
            &config.moltbook,
            config.moltbook.timeout_secs,
            config.moltbook.max_response_bytes,
        ) {
            Ok(register_client) => {
                gatekeeper.register(Arc::new(crate::tools::moltbook::MoltbookRegisterTool {
                    client: Arc::new(register_client),
                }));
            }
            Err(e) => {
                tracing::warn!(error = %e, "Moltbook register tool not available");
            }
        }

        match crate::tools::moltbook::MoltbookClient::authenticated(
            &config.moltbook,
            config.moltbook.timeout_secs,
            config.moltbook.max_response_bytes,
        )
        .await
        {
            Ok(client) => {
                let client = Arc::new(client);
                gatekeeper.register(Arc::new(crate::tools::moltbook::MoltbookStatusTool {
                    client: client.clone(),
                }));
                gatekeeper.register(Arc::new(crate::tools::moltbook::MoltbookHomeTool {
                    client: client.clone(),
                }));
                gatekeeper.register(Arc::new(crate::tools::moltbook::MoltbookFeedTool {
                    client: client.clone(),
                }));
                gatekeeper.register(Arc::new(crate::tools::moltbook::MoltbookSearchTool {
                    client: client.clone(),
                }));
                gatekeeper.register(Arc::new(crate::tools::moltbook::MoltbookCommentsTool {
                    client: client.clone(),
                }));
                gatekeeper.register(Arc::new(crate::tools::moltbook::MoltbookCommentTool {
                    client: client.clone(),
                }));
                gatekeeper.register(Arc::new(crate::tools::moltbook::MoltbookVoteTool {
                    client: client.clone(),
                }));
                gatekeeper.register(Arc::new(crate::tools::moltbook::MoltbookPostTool {
                    client: client.clone(),
                }));
                gatekeeper.register(Arc::new(crate::tools::moltbook::MoltbookVerifyTool {
                    client: client.clone(),
                }));
                gatekeeper.register(Arc::new(
                    crate::tools::moltbook::MoltbookNotificationsReadTool {
                        client: client.clone(),
                    },
                ));
                gatekeeper.register(Arc::new(crate::tools::moltbook::MoltbookDmTool { client }));
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Moltbook authenticated tools not registered; configure credentials to enable them"
                );
            }
        }
    }

    let taglist_cache = crate::tools::vault::TaglistCache::into_arc();
    {
        let workspace_for_build = workspace_root.clone();
        tokio::spawn(async move {
            match crate::tools::vault::taglist_index::build_and_persist(&workspace_for_build).await
            {
                Ok(snap) => tracing::info!(
                    event = "fcp.vault.taglist.startup_built",
                    note_count = snap.note_count,
                    tag_count = snap.tags.len(),
                    "vault:taglist startup snapshot built"
                ),
                Err(e) => tracing::warn!(
                    error = %e,
                    "vault:taglist startup build failed (will retry on first call)"
                ),
            }
        });
    }
    gatekeeper.register(Arc::new(crate::tools::vault::VaultReadTool {
        workspace_root: workspace_root.clone(),
        read_limit,
    }));
    gatekeeper.register(Arc::new(crate::tools::vault::VaultWriteTool {
        workspace_root: workspace_root.clone(),
        max_content_chars: config.num_ctx * 3,
        taglist_cache: Arc::clone(&taglist_cache),
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
    gatekeeper.register(Arc::new(crate::tools::vault::VaultTaglistTool {
        workspace_root: workspace_root.clone(),
        cache: Arc::clone(&taglist_cache),
    }));
    gatekeeper.register(Arc::new(crate::tools::skills::SkillsListTool {
        workspace_root: workspace_root.clone(),
    }));
    gatekeeper.register(Arc::new(crate::tools::skills::SkillsReadTool {
        workspace_root: workspace_root.clone(),
    }));
    gatekeeper.register(Arc::new(crate::tools::skills::SkillsCreateTool {
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
    gatekeeper.register(Arc::new(crate::tools::agenda::AgendaRemindSelfTool {
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
    let _ = presentation_tx
        .send(SessionEvent::SystemError(
            "[startup] Preparing web stack (browser39, vault operator files)...".into(),
        ))
        .await;
    let browser39_probe = crate::tools::web::bootstrap::ensure_web_stack_ready(
        &workspace_root,
        &config.web_fetch_user_agent,
        config.web.require_browser39,
    )
    .await?;
    if let Some(probe) = &browser39_probe {
        let _ = presentation_tx
            .send(SessionEvent::SystemError(format!(
                "[startup] browser39 ready: {} ({})",
                probe.binary, probe.version_line
            )))
            .await;
        tracing::info!(
            binary = %probe.binary,
            version = %probe.version_line,
            "browser39 verified at chat startup"
        );
    } else {
        tracing::info!("browser39 binary probe skipped (web.require_browser39 = false)");
    }

    let web_ledger = Arc::new(tokio::sync::Mutex::new(
        crate::tools::web::WebSessionLedger::load_from_vault(&workspace_root, &config.web)
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "web session ledger load failed; starting empty");
                crate::tools::web::WebSessionLedger::new()
            }),
    ));
    {
        let mut ledger = web_ledger.lock().await;
        ledger.reset_session();
    }
    let web_fetcher = crate::tools::web::WebFetcherKind::Browser39 {
        binary: crate::tools::web::bootstrap::resolve_browser39_binary(),
    };
    let web_ctx = crate::tools::web::WebToolContext::from_config(
        &config,
        &workspace_root,
        web_ledger.clone(),
        web_fetcher,
        effective_web_fetch_max_bytes,
    );
    gatekeeper.register(Arc::new(crate::tools::web::WebFetchTool {
        ctx: web_ctx.clone(),
    }));
    if config.web.search_enabled {
        gatekeeper.register(Arc::new(crate::tools::web::WebSearchTool {
            ctx: web_ctx.clone(),
        }));
    } else {
        tracing::info!("web:search disabled by config — not registered");
    }
    gatekeeper.register(Arc::new(crate::tools::web::WebFindTool {
        ctx: web_ctx.clone(),
        max_snippet_chars: (read_limit.max(512) / 3).clamp(300, 900),
        max_total_chars: (read_limit.max(512) / 2).clamp(1000, 2500),
    }));
    if config.news_today_enabled {
        gatekeeper.register(Arc::new(crate::tools::news::NewsTodayTool::new(
            web_ctx,
            crate::tools::news::NewsTodayConfigSnapshot {
                site_base: config.news_today_site_base.clone(),
                default_homepage: config.news_today_default_homepage.clone(),
                max_headlines_default: config.news_today_max_headlines_default,
                deep_fetch_max_default: config.news_today_deep_fetch_max_default,
            },
        )));
    } else {
        tracing::info!("news:today disabled by config — not registered");
    }
    gatekeeper.register(Arc::new(crate::tools::system::SystemHealthTool {
        config: config.clone(),
    }));

    if config.vision.enabled {
        gatekeeper.register(Arc::new(crate::tools::vision::VisionSeeTool {
            config: config.clone(),
            workspace_root: workspace_root.clone(),
        }));
    } else {
        tracing::info!("vision:see disabled by config — not registered");
    }

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

    if let Some(auth) = crate::util::google_workspace::workspace_auth(&config.google).await? {
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
        embed_provider,
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
        semantic_arc.clone(),
        config.snapshot_interval_secs,
        cancel_token.clone(),
        config.clone(),
        promotion_suppressed_during_step.clone(),
    );

    if config.vault_reindex_on_write {
        if let Some(semantic) = semantic_arc.clone() {
            let debounce =
                std::time::Duration::from_millis(config.vault_watch.debounce_ms.max(1));
            crate::memory::reindex_watch::spawn_vault_semantic_reindex_watch(
                cancel_token.child_token(),
                debounce,
                workspace_root.clone(),
                semantic,
            );
        }
    }

    let context_view_hints =
        gatekeeper.merge_context_view_hints(&config.optimize_context_tool_overrides);
    let context_view = crate::orchestrator::context::ContextViewSettings {
        enabled: config.optimize_context,
        default_snippet_chars: config.optimize_context_max_tool_snippet_chars,
        assistant_compact: config.optimize_context_assistant_compact,
        full_tool_schemas_in_llm_view: config.optimize_context_full_tool_schemas,
        omit_resolved_tool_recovery: config.optimize_context_omit_resolved_tool_recovery,
        assistant_non_json_placeholder: config.optimize_context_assistant_non_json_placeholder,
        hints: Arc::new(context_view_hints),
    };

    if config.is_llamacpp() {
        let tool_names = gatekeeper.registered_tool_names();
        let mut typed_count: usize = 0;
        let mut fallback_count: usize = 0;

        let entries: Vec<crate::engine::grammar::ToolGrammarEntry> = tool_names
            .iter()
            .map(|name| {
                let per_tool_rules = gatekeeper
                    .parameters_root_schema_for(name)
                    .and_then(|schema| {
                        crate::engine::grammar::schema_to_gbnf_rule(name, &schema)
                    })
                    .map(|(_rule_name, rules)| rules);

                if per_tool_rules.is_some() {
                    typed_count += 1;
                } else {
                    fallback_count += 1;
                }

                crate::engine::grammar::ToolGrammarEntry {
                    name: name.clone(),
                    per_tool_rules,
                }
            })
            .collect();

        let grammar =
            crate::engine::grammar::compile_fcp_envelope_grammar_dynamic(&entries);
        tracing::info!(
            tool_count = tool_names.len(),
            typed_count,
            fallback_count,
            grammar_len = grammar.len(),
            "Compiled dynamic per-tool GBNF grammar for llama.cpp"
        );
        engine.set_grammar(grammar);
    }

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
        Some(token_metrics_rx.clone()),
        Some(web_ledger),
        semantic_arc,
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
    let workspace_root_loop = workspace_root.clone();
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
                            let is_moltbook =
                                trimmed.to_ascii_lowercase().contains("moltbook");
                            let content = if is_moltbook {
                                format!(
                                    "{}{}\n\n\
                                    [MOLTBOOK_CYCLE task_id={} alarm_id={}]\n\
                                    Follow the Moltbook Browse Session Protocol in your identity.\n\
                                    Use clock:now to check the time budget. Include moltbook:search sometimes for discovery; welcome newcomers after reading threads.\n\
                                    If expired, summarize and call agenda:remove with task_id above.",
                                    SYSTEM_ALARM_PREFIX,
                                    trimmed,
                                    agenda_task_id,
                                    alarm_record_id,
                                )
                            } else {
                                let late_note = if seconds_late > 60 {
                                    format!(" (~{} min late)", seconds_late / 60)
                                } else {
                                    String::new()
                                };
                                format!(
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
                                )
                            };
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
                        UserAction::AgendaSelfPrompt {
                            agenda_task_id,
                            label,
                            plan,
                            checklist,
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
                            let checklist_block = if checklist.is_empty() {
                                "- [ ] (no checklist provided; execute plan directly)".to_string()
                            } else {
                                checklist
                                    .iter()
                                    .map(|step| format!("- [ ] {}", step))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            };
                            let content = format!(
                                "{}{}{}\n\n\
                                [SELF_REMINDER_PLAN]\n\
                                {}\n\
                                [/SELF_REMINDER_PLAN]\n\n\
                                [SELF_REMINDER_CHECKLIST]\n\
                                {}\n\
                                [/SELF_REMINDER_CHECKLIST]\n\n\
                                Execute this plan autonomously now. When finished, call agenda:complete for task_id below with a concise result_summary.\n\
                                If you need another cycle, call agenda:remind_self with the same task_id and an updated plan/checklist.\n\n\
                                [AGENDA_SELF task_id={} alarm_id={} late_sec={}]",
                                SYSTEM_SELF_REMINDER_PREFIX,
                                trimmed,
                                late_note,
                                plan.trim(),
                                checklist_block,
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
                            tracing::info!("Agenda self-reminder alarm turn");
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
                                    image: None,
                                    audio: None,
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
                if ing.image.is_some() && !orchestrator.config.vision.enabled {
                    tracing::warn!(
                        target: "fcp.vision",
                        "Rejected user ingress with image attachment while vision disabled"
                    );
                    let _ = presentation_tx_err
                        .send(SessionEvent::SystemError(
                            "[ui] Vision is disabled in config — image attachment rejected."
                                .into(),
                        ))
                        .await;
                    continue;
                }
                if ing.audio.is_some() && !orchestrator.config.audio.enabled {
                    tracing::warn!(
                        target: "fcp.audio",
                        "Rejected user ingress with audio attachment while audio disabled"
                    );
                    let _ = presentation_tx_err
                        .send(SessionEvent::SystemError(
                            "[ui] Voice ingress is disabled in config — audio attachment rejected."
                                .into(),
                        ))
                        .await;
                    continue;
                }

                let (display, for_model) = if let Some(audio_att) = &ing.audio {
                    orchestrator.state = crate::orchestrator::state::AgentState::Chat;
                    orchestrator.activity_line = Some("Transcribing voice…".into());
                    orchestrator.broadcast_state().await;
                    let _ = presentation_tx_err
                        .send(SessionEvent::SystemError(
                            "[ui] Transcribing voice…".into(),
                        ))
                        .await;
                    match crate::util::audio::transcribe_audio(
                        &orchestrator.config,
                        &workspace_root_loop,
                        &audio_att.relative_path,
                    )
                    .await
                    {
                        Ok(transcript) => {
                            let t = transcript.trim();
                            if t.is_empty() {
                                tracing::warn!(
                                    target: "fcp.audio",
                                    path = %audio_att.relative_path,
                                    "STT returned empty transcript"
                                );
                                let _ = presentation_tx_err
                                    .send(SessionEvent::SystemError(
                                        "[ui] Could not transcribe audio — empty result."
                                            .into(),
                                    ))
                                    .await;
                                orchestrator.activity_line = None;
                                orchestrator.state = crate::orchestrator::state::AgentState::Idle;
                                orchestrator.broadcast_state().await;
                                continue;
                            }
                            let caption = ing.display.trim();
                            let merged = if caption.is_empty()
                                || caption == "(voice message)"
                            {
                                t.to_string()
                            } else {
                                format!("{t}\n\n{caption}")
                            };
                            tracing::info!(
                                target: "fcp.audio",
                                path = %audio_att.relative_path,
                                transcript_len = merged.len(),
                                "voice transcribed for orchestrator turn"
                            );
                            (merged.clone(), merged)
                        }
                        Err(e) => {
                            tracing::warn!(
                                target: "fcp.audio",
                                error = %e,
                                "STT failed"
                            );
                            orchestrator.activity_line = None;
                            orchestrator.state = crate::orchestrator::state::AgentState::Idle;
                            orchestrator.broadcast_state().await;
                            let _ = presentation_tx_err
                                .send(SessionEvent::SystemError(format!(
                                    "[ui] Voice transcription failed: {e}"
                                )))
                                .await;
                            continue;
                        }
                    }
                } else {
                    (
                        ing.display.clone(),
                        build_user_for_model(&ing),
                    )
                };

                tracing::info!(
                    msg_len = for_model.len(),
                    queued = pending_inputs.len(),
                    source = ?ing.source,
                    has_image = ing.image.is_some(),
                    has_audio = ing.audio.is_some(),
                    "User input received"
                );
                if presentation_tx_err
                    .send(SessionEvent::UserTranscriptLine {
                        source: ing.source,
                        body: display,
                        image: ing.image.clone(),
                        audio: ing.audio.clone(),
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
