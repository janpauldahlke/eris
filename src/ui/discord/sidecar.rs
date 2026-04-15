//! Start Serenity and a non-blocking outbound loop for assistant lines from the presentation mux.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use serenity::builder::CreateMessage;
use serenity::client::Client;
use serenity::http::Typing;
use serenity::model::gateway::GatewayIntents;
use serenity::model::id::ApplicationId;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::config::AppConfig;
use crate::executive::error::{FcpError, Result};
use crate::presentation::UserAction;
use super::format::chunk_discord_content;
use super::handler::DiscordHandler;
use super::DiscordReadySignal;
use super::DiscordTypingCtl;

const DISCORD_CONTENT_CHAR_BUDGET: usize = 1990;
const READY_TIMEOUT: Duration = Duration::from_secs(90);

/// Runs the Discord gateway client and drains `outbound_rx` until cancelled or the channel closes.
///
/// `typing_ctl_rx` carries [`DiscordTypingCtl`] only; it never touches web or TUI [`SessionEvent`]s.
pub async fn run_discord_sidecar(
    config: Arc<AppConfig>,
    user_action_tx: mpsc::Sender<UserAction>,
    mut outbound_rx: mpsc::Receiver<String>,
    mut typing_ctl_rx: mpsc::Receiver<DiscordTypingCtl>,
    cancel_token: CancellationToken,
) -> Result<()> {
    let token = config.resolved_discord_bot_token()?;
    let application_id = config.discord.application_id.ok_or_else(|| {
        FcpError::Config("Discord enabled: missing discord.application_id".into())
    })?;
    let (ready_tx, mut ready_rx) = mpsc::channel::<DiscordReadySignal>(1);

    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;

    let listen_channel_id = Arc::new(tokio::sync::RwLock::new(None));
    let listen_ready_sent = Arc::new(AtomicBool::new(false));
    let handler = DiscordHandler {
        user_action_tx: user_action_tx.clone(),
        config: config.clone(),
        ready_tx,
        listen_channel_id,
        listen_ready_sent,
    };

    let mut client = Client::builder(&token, intents)
        .application_id(ApplicationId::new(application_id))
        .event_handler(handler)
        .await
        .map_err(|e| FcpError::Config(format!("Discord client build failed: {e}")))?;

    let shard_manager = client.shard_manager.clone();

    let start_jh = tokio::spawn(async move {
        tracing::info!(event = "fcp.discord.shard_starting", "Discord gateway client starting");
        if let Err(e) = client.start().await {
            tracing::error!(
                event = "fcp.discord.client_start_failed",
                error = %e,
                "Discord client.start returned error"
            );
        }
    });

    let ready_wait = tokio::time::timeout(READY_TIMEOUT, ready_rx.recv());
    let first = ready_wait
        .await
        .map_err(|_| FcpError::Config("Discord ready timed out".into()))?
        .ok_or_else(|| FcpError::Config("Discord ready channel closed before READY".into()))?;

    let (http, channel_id) = match first {
        DiscordReadySignal::Connected { http, channel_id } => (http, channel_id),
        DiscordReadySignal::Failed { message } => {
            let _ = shard_manager.shutdown_all().await;
            start_jh.abort();
            let _ = start_jh.await;
            return Err(FcpError::Config(format!("Discord listen channel: {message}")));
        }
    };

    tracing::info!(
        event = "fcp.discord.shard_connected",
        channel_id = %channel_id,
        "Discord outbound worker running"
    );

    let mut active_typing: Option<Typing> = None;
    let mut typing_ctl_open = true;

    loop {
        tokio::select! {
            biased;
            _ = cancel_token.cancelled() => {
                drop(active_typing.take());
                tracing::info!(event = "fcp.discord.shutdown", "Discord sidecar shutting down");
                break;
            }
            ctl = typing_ctl_rx.recv(), if typing_ctl_open => {
                match ctl {
                    Some(DiscordTypingCtl::StartPulse) => {
                        drop(active_typing.take());
                        active_typing = Some(Typing::start(http.clone(), channel_id));
                        tracing::debug!(event = "fcp.discord.typing_pulse_started", "Discord typing pulse armed");
                    }
                    Some(DiscordTypingCtl::StopPulse) => {
                        drop(active_typing.take());
                        tracing::debug!(event = "fcp.discord.typing_pulse_stopped", "Discord typing pulse cleared");
                    }
                    None => {
                        typing_ctl_open = false;
                        drop(active_typing.take());
                        tracing::debug!(
                            event = "fcp.discord.typing_ctl_closed",
                            "Discord typing control channel closed"
                        );
                    }
                }
            }
            maybe = outbound_rx.recv() => {
                let Some(text) = maybe else {
                    drop(active_typing.take());
                    tracing::debug!(event = "fcp.discord.outbound_rx_closed", "Discord outbound queue closed");
                    break;
                };
                let parts: Vec<String> = chunk_discord_content(&text, DISCORD_CONTENT_CHAR_BUDGET)
                    .into_iter()
                    .filter(|p| !p.is_empty())
                    .collect();
                let mut parts_posted: usize = 0;
                for part in &parts {
                    match channel_id
                        .send_message(http.as_ref(), CreateMessage::new().content(part))
                        .await
                    {
                        Ok(_) => {
                            parts_posted = parts_posted.saturating_add(1);
                            tracing::debug!(
                                event = "fcp.discord.send_ok",
                                part_len = part.len(),
                                "Posted assistant segment to Discord"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                event = "fcp.discord.send_failed",
                                error = %e,
                                part_len = part.len(),
                                "Discord send_message failed"
                            );
                        }
                    }
                }
                if !parts.is_empty() && parts_posted == parts.len() {
                    tracing::info!(
                        event = "fcp.discord.posted",
                        parts = parts.len(),
                        total_len = text.len(),
                        "Posted assistant message to Discord"
                    );
                } else if !parts.is_empty() && parts_posted < parts.len() {
                    tracing::warn!(
                        event = "fcp.discord.post_partial",
                        parts_ok = parts_posted,
                        parts_total = parts.len(),
                        total_len = text.len(),
                        "Posted only part of assistant message to Discord (see send_failed)"
                    );
                }
            }
        }
    }

    drop(active_typing.take());
    shard_manager.shutdown_all().await;
    start_jh.abort();
    let _ = start_jh.await;

    Ok(())
}
