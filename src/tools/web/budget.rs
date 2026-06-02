//! Content byte/chunk caps for persisted web mission pages and browser39 pagination.

use crate::config::AppConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebBudget {
    /// Total markdown bytes retained for one fetch (before chunk split).
    pub max_bytes: usize,
    /// Max characters per persisted vault chunk under `20_Discourse/web/missions/.../chunks/`.
    pub chunk_chars: usize,
    /// Slice of the first chunk used for `preview_head` in the fetch receipt (receipt also capped at 600).
    pub preview_chars: usize,
    pub max_chunks: usize,
    /// Approximate max_tokens per browser39 pagination request.
    pub page_max_tokens: u32,
}

impl WebBudget {
    pub fn from_config(config: &AppConfig, effective_max_bytes: usize) -> Self {
        Self::from_parts(
            config.num_ctx,
            config.vault_read_ratio,
            config.resolved_web_fetch_chunk_chars(),
            effective_max_bytes,
        )
    }

    pub fn from_parts(
        num_ctx: usize,
        vault_read_ratio: f32,
        web_fetch_chunk_chars: usize,
        effective_max_bytes: usize,
    ) -> Self {
        let chunk_chars = web_fetch_chunk_chars.max(512);
        let read_limit = (num_ctx.max(1) as f32 * vault_read_ratio) as usize;
        let preview_chars = (read_limit / 2).max(256);
        let max_bytes = effective_max_bytes
            .min(chunk_chars.saturating_mul(6))
            .max(chunk_chars);
        let max_chunks = (max_bytes / chunk_chars.max(1)).max(1).min(64);
        let page_max_tokens = (chunk_chars / 4).clamp(512, 8192) as u32;
        Self {
            max_bytes,
            chunk_chars,
            preview_chars,
            max_chunks,
            page_max_tokens,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_chunk_splits_wide_body() {
        let budget = WebBudget::from_parts(64_000, 0.5, 57_600, 786_432);
        assert_eq!(budget.chunk_chars, 57_600);
        assert!(budget.max_bytes >= 57_600);
        assert_eq!(budget.preview_chars, 16_000);
    }

    #[test]
    fn preview_chars_stays_on_vault_read_ratio_not_chunk_size() {
        let budget = WebBudget::from_parts(8192, 0.5, 7372, 50_000);
        assert_eq!(budget.chunk_chars, 7372);
        assert_eq!(budget.preview_chars, 2048);
    }
}
