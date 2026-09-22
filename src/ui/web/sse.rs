//! SSE stream: [`broadcast::Receiver`] → Axum [`Event`] stream.

use std::convert::Infallible;
use std::time::Duration;

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
    tracing::debug!(
        event = "fcp.web.sse.subscribe",
        "SSE client subscribed to session events"
    );
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
    // 15s default is fine for most browsers; 10s is safer behind flaky local proxies /
    // hybrid-GPU desktop freezes that pause the tab briefly.
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(10)))
}
