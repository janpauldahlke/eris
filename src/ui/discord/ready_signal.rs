//! Gateway `ready` handshake for the outbound REST worker (capacity 1).

use std::sync::Arc;

use serenity::http::Http;
use serenity::model::id::ChannelId;

/// Handed from the gateway `ready` path to the outbound REST worker (capacity 1).
pub(crate) enum DiscordReadySignal {
    Connected {
        http: Arc<Http>,
        channel_id: ChannelId,
    },
    Failed {
        message: String,
    },
}
