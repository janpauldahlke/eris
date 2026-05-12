//! Last-reported LLM token counts (Ollama `prompt_eval_count` / `eval_count`, or llama-server
//! `usage.*_tokens`), published on every successful `generate` completion. Use [`channel`] for a
//! shared [`tokio::sync::watch`] pair so UI or other tasks can read the latest snapshot without
//! `Arc<Mutex<_>>`.

use std::time::{SystemTime, UNIX_EPOCH};

/// Snapshot of token usage for the **most recent** completed chat generation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LlmTokenSnapshot {
    pub prompt_tokens: usize,
    pub generated_tokens: usize,
    /// Wall-clock ms since UNIX epoch when this snapshot was recorded.
    pub recorded_at_unix_ms: u64,
    /// Wall-clock ms spent inside the engine for the last finished generation (0 if unknown).
    pub last_generation_ms: u64,
    /// Completion throughput for the last generation: \((generated\_tokens / s) × 1000\) as `u32`.
    /// Display as `last_tps_milli / 1000.0` tokens/s. Zero means unknown or a zero-duration sample.
    pub last_tps_milli: u32,
    /// EWMA of the same quantity as [`Self::last_tps_milli`] (×1000 fixed-point). Zero until first
    /// non-zero sample.
    pub ewma_tps_milli: u32,
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

/// Publish counts from the engine after a completed generation (or zeros if missing).
pub fn publish(
    tx: &Option<tokio::sync::watch::Sender<LlmTokenSnapshot>>,
    prompt_tokens: usize,
    generated_tokens: usize,
    generation_ms: u64,
) {
    let Some(tx) = tx else {
        return;
    };
    let recorded_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let last_tps_milli = if generation_ms > 0 && generated_tokens > 0 {
        let v = (generated_tokens as u128)
            .saturating_mul(1_000_000)
            / u128::from(generation_ms.max(1));
        u32::try_from(v.min(u128::from(u32::MAX))).unwrap_or(u32::MAX)
    } else {
        0
    };
    let prev = tx.borrow();
    let ewma_tps_milli = if last_tps_milli == 0 {
        prev.ewma_tps_milli
    } else if prev.ewma_tps_milli == 0 {
        last_tps_milli
    } else {
        let a = u64::from(prev.ewma_tps_milli);
        let b = u64::from(last_tps_milli);
        u32::try_from((a * 85 + b * 15) / 100).unwrap_or(u32::MAX)
    };
    drop(prev);
    let snap = LlmTokenSnapshot {
        prompt_tokens,
        generated_tokens,
        recorded_at_unix_ms,
        last_generation_ms: generation_ms,
        last_tps_milli,
        ewma_tps_milli,
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
        publish(&Some(tx), 10, 5, 1000);
        let r = TokenMetricsReader::new(rx);
        let s = r.snapshot();
        assert_eq!(s.prompt_tokens, 10);
        assert_eq!(s.generated_tokens, 5);
        assert_eq!(s.total(), 15);
        assert_eq!(s.last_generation_ms, 1000);
        assert_eq!(s.last_tps_milli, 5000);
        assert_eq!(s.ewma_tps_milli, 5000);
    }
}
