//! Content byte/chunk caps derived from context window settings.

use crate::config::AppConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebBudget {
    pub max_bytes: usize,
    pub chunk_chars: usize,
    pub preview_chars: usize,
    pub max_chunks: usize,
    /// Approximate max_tokens per browser39 pagination request.
    pub page_max_tokens: u32,
}

impl WebBudget {
    pub fn from_config(config: &AppConfig, effective_max_bytes: usize) -> Self {
        Self::from_parts(config.num_ctx, config.vault_read_ratio, effective_max_bytes)
    }

    pub fn from_parts(num_ctx: usize, vault_read_ratio: f32, effective_max_bytes: usize) -> Self {
        let read_limit = (num_ctx as f32 * vault_read_ratio) as usize;
        let chunk_chars = read_limit.max(512);
        let preview_chars = (chunk_chars / 2).max(256);
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
