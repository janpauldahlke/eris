//! SSE stream: [`broadcast::Receiver`] → Axum [`Event`] stream.

use std::convert::Infallible;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

use super::WebAppState;

pub async fn session_events_sse(
    State(state): State<WebAppState>,
) -> Sse<impl futures::stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.events_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|item| match item {
        Ok(ev) => match serde_json::to_string(&ev) {
            Ok(json) => Some(Ok(Event::default().data(json))),
            Err(e) => {
                tracing::warn!(
                    event = "fcp.web.sse.serialize_failed",
                    error = %e,
                    "failed to serialize SessionEvent for SSE"
                );
                None
            }
        },
        Err(BroadcastStreamRecvError::Lagged(n)) => {
            tracing::debug!(
                event = "fcp.web.sse.client_lagged",
                skipped = n,
                "SSE client lagged; skipped events"
            );
            None
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}
