//! Last-reported LLM token counts from Ollama (`prompt_eval_count` / `eval_count`), published on
//! every successful `generate` completion. Use [`channel`] for a shared [`tokio::sync::watch`]
//! pair so UI or other tasks can read the latest snapshot without `Arc<Mutex<_>>`.

use std::time::{SystemTime, UNIX_EPOCH};

/// Snapshot of token usage for the **most recent** completed Ollama chat generation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LlmTokenSnapshot {
    pub prompt_tokens: usize,
    pub generated_tokens: usize,
    /// Wall-clock ms since UNIX epoch when this snapshot was recorded.
    pub recorded_at_unix_ms: u64,
}

impl LlmTokenSnapshot {
    #[must_use]
    pub fn total(&self) -> usize {
        self.prompt_tokens.saturating_add(self.generated_tokens)
    }
}

/// Create a watch pair: engine holds the sender; subscribers clone the receiver.
#[must_use]
pub fn channel() -> (
    tokio::sync::watch::Sender<LlmTokenSnapshot>,
    tokio::sync::watch::Receiver<LlmTokenSnapshot>,
) {
    tokio::sync::watch::channel(LlmTokenSnapshot::default())
}

/// Publish counts from the engine after Ollama returns `final_data` (or zeros if missing).
pub fn publish(
    tx: &Option<tokio::sync::watch::Sender<LlmTokenSnapshot>>,
    prompt_tokens: usize,
    generated_tokens: usize,
) {
    let Some(tx) = tx else {
        return;
    };
    let recorded_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let snap = LlmTokenSnapshot {
        prompt_tokens,
        generated_tokens,
        recorded_at_unix_ms,
    };
    let _ = tx.send(snap);
}

/// Read-only handle for tests and helpers: wraps a receiver and exposes the latest snapshot.
#[derive(Clone)]
pub struct TokenMetricsReader {
    rx: tokio::sync::watch::Receiver<LlmTokenSnapshot>,
}

impl TokenMetricsReader {
    #[must_use]
    pub fn new(rx: tokio::sync::watch::Receiver<LlmTokenSnapshot>) -> Self {
        Self { rx }
    }

    #[must_use]
    pub fn snapshot(&self) -> LlmTokenSnapshot {
        self.rx.borrow().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_updates_watch() {
        let (tx, rx) = channel();
        publish(&Some(tx), 10, 5);
        let r = TokenMetricsReader::new(rx);
        let s = r.snapshot();
        assert_eq!(s.prompt_tokens, 10);
        assert_eq!(s.generated_tokens, 5);
        assert_eq!(s.total(), 15);
    }
}
