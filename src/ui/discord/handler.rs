//! Serenity gateway event handler: resolve listen channel, forward Discord messages to `UserAction`.
//!
//! Listen-channel resolution runs on [`EventHandler::cache_ready`], not [`EventHandler::ready`]:
//! Serenity fills guild channels from `GUILD_CREATE` after the READY payload; scanning the cache
//! in `ready` often sees an empty channel list.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serenity::async_trait;
use serenity::model::channel::ChannelType;
use serenity::model::gateway::Ready;
use serenity::model::id::{ChannelId, GuildId};
use serenity::model::prelude::Message;
use serenity::prelude::{Context, EventHandler};
use tokio::sync::{RwLock, mpsc};

use crate::config::AppConfig;
use crate::presentation::{InputSource, UserAction, UserIngress};

use super::DiscordReadySignal;

pub struct DiscordHandler {
    pub user_action_tx: mpsc::Sender<UserAction>,
    pub config: Arc<AppConfig>,
    pub ready_tx: mpsc::Sender<DiscordReadySignal>,
    /// Filled after [`EventHandler::cache_ready`]; message handler compares without full cache scans.
    pub listen_channel_id: Arc<RwLock<Option<ChannelId>>>,
    /// Ensures we send at most one [`DiscordReadySignal`] per gateway session (`cache_ready` may repeat on reconnect).
    pub listen_ready_sent: Arc<AtomicBool>,
}

fn resolve_listen_channel(ctx: &Context, config: &AppConfig) -> Result<ChannelId, String> {
    if let Some(id) = config.discord.channel_id {
        return Ok(ChannelId::new(id));
    }
    let name = config
        .discord
        .channel_name
        .as_deref()
        .map(str::trim)
        .unwrap_or("");
    if name.is_empty() {
        return Err("discord.channel_name is empty".into());
    }
    let name_norm = name.to_lowercase();
    let guild_ids: Vec<_> = ctx.cache.guilds();
    let guild_count = guild_ids.len();
    if guild_ids.is_empty() {
        return Err(
            "Discord cache shows no servers: invite the bot with an OAuth2 URL that includes the `bot` scope (and pick the target server)"
                .into(),
        );
    }
    let mut text_like_names: Vec<String> = Vec::new();
    for guild_id in guild_ids {
        let Some(guild) = ctx.cache.guild(guild_id) else {
            continue;
        };
        for ch in guild.channels.values() {
            let is_text_like = matches!(ch.kind, ChannelType::Text | ChannelType::News);
            if !is_text_like {
                continue;
            }
            if ch.name.to_lowercase() == name_norm {
                return Ok(ch.id);
            }
            if text_like_names.len() < 48 {
                text_like_names.push(ch.name.clone());
            }
        }
    }
    let preview = if text_like_names.is_empty() {
        "none in cache (bot may lack View Channel on categories, or channels are not Text/Announcement types)"
            .into()
    } else {
        text_like_names.join(", ")
    };
    Err(format!(
        "no text/news channel matching {:?} (case-insensitive) in {} server(s). Visible text/news names (sample): {}",
        name, guild_count, preview
    ))
}

#[async_trait]
impl EventHandler for DiscordHandler {
    async fn ready(&self, _ctx: Context, _data: Ready) {
        tracing::info!(
            event = "fcp.discord.gateway_session_ready",
            "Discord gateway READY; waiting for cache_ready before resolving listen channel"
        );
    }

    async fn cache_ready(&self, ctx: Context, _guilds: Vec<GuildId>) {
        if self.listen_ready_sent.swap(true, Ordering::SeqCst) {
            tracing::debug!(
                event = "fcp.discord.cache_ready_skipped",
                "Ignoring duplicate cache_ready (reconnect or duplicate dispatch)"
            );
            return;
        }

        let (signal, resolved_ok) = match resolve_listen_channel(&ctx, &self.config) {
            Ok(channel_id) => {
                *self.listen_channel_id.write().await = Some(channel_id);
                (
                    DiscordReadySignal::Connected {
                        http: ctx.http.clone(),
                        channel_id,
                    },
                    true,
                )
            }
            Err(message) => {
                tracing::warn!(
                    event = "fcp.discord.listen_channel_unresolved",
                    reason = %message,
                    "Discord cache_ready fired but listen channel could not be matched; sidecar will exit until config is fixed"
                );
                (DiscordReadySignal::Failed { message }, false)
            }
        };
        if self.ready_tx.try_send(signal).is_err() {
            tracing::error!(
                event = "fcp.discord.ready_signal_dropped",
                "Discord ready signal channel full or closed"
            );
        } else if resolved_ok {
            tracing::info!(
                event = "fcp.discord.ready",
                "Discord listen channel resolved after cache_ready"
            );
        }
    }

    async fn message(&self, _ctx: Context, msg: Message) {
        let Some(target) = *self.listen_channel_id.read().await else {
            tracing::debug!(
                event = "fcp.discord.message_ignored",
                reason = "listen_channel_pending",
                "Ignoring Discord message until listen channel is resolved"
            );
            return;
        };
        if msg.channel_id != target {
            tracing::debug!(
                event = "fcp.discord.message_ignored",
                reason = "wrong_channel",
                "Ignoring Discord message outside listen channel"
            );
            return;
        }
        if msg.author.bot {
            tracing::debug!(
                event = "fcp.discord.message_ignored",
                reason = "author_bot",
                "Ignoring bot-authored Discord message"
            );
            return;
        }

        let trimmed = msg.content.trim();
        if trimmed.eq_ignore_ascii_case("!cancel") {
            if self
                .user_action_tx
                .try_send(UserAction::CancelCurrentTurn)
                .is_err()
            {
                tracing::warn!(
                    event = "fcp.discord.submit_dropped",
                    reason = "cancel_channel",
                    "Dropped !cancel: user action channel full or closed"
                );
            } else {
                tracing::debug!(
                    event = "fcp.discord.cancel_queued",
                    "Queued CancelCurrentTurn from Discord"
                );
            }
            return;
        }

        if trimmed.is_empty() {
            if msg.content.is_empty() && msg.attachments.is_empty() {
                tracing::warn!(
                    event = "fcp.discord.message_empty_content",
                    channel_id = %msg.channel_id,
                    "Discord message has no text in the listen channel; if you typed text, enable Message Content Intent (Developer Portal → Bot → Privileged Gateway Intents)"
                );
            }
            return;
        }

        let author = msg
            .author
            .global_name
            .as_deref()
            .unwrap_or(msg.author.name.as_str());
        let tagged = format!("[Input via Discord from @{author}]\n\n{}", msg.content);
        let ingress = UserIngress {
            source: InputSource::Discord,
            display: msg.content.clone(),
            for_model: Some(tagged),
        };
        if self
            .user_action_tx
            .try_send(UserAction::SubmitIngress(ingress))
            .is_err()
        {
            tracing::warn!(
                event = "fcp.discord.submit_dropped",
                reason = "channel_full",
                message_len = msg.content.len(),
                "Dropped Discord submit: user action channel full or closed"
            );
        } else {
            tracing::info!(
                event = "fcp.discord.submit_queued",
                message_len = msg.content.len(),
                "Queued Submit from Discord to session"
            );
        }
    }
}
