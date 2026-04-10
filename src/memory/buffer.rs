//! Shared ephemeral **big blob** staging: chunking, JSON shape, and paging.
//! Used by `vault:read` (large files), `web:fetch`, and `ephemeral:buffer_page`.

use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::executive::error::{FcpError, Result};
use crate::ingest::{bound_chunks_and_preview, truncate_char_boundary};
use crate::memory::ephemeral::CacheValue;

/// Tag on [`crate::memory::ephemeral::CacheValue`] for unified buffer rows.
pub const TAG_EPHEMERAL_BUFFER: &str = "ephemeral_buffer";
/// Large vault read staged via [`crate::memory::buffer`].
pub const TAG_VAULT_READ_BUFFER: &str = "vault_read_buffer";

/// JSON in `CacheValue.data`. Field serializes as `url` for compatibility with existing web artifacts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BufferedBlob {
    #[serde(rename = "url")]
    pub source: String,
    pub chunks: Vec<String>,
}

/// Caps derived from [`AppConfig`] (`num_ctx`, `vault_read_ratio`, `web_fetch_max_bytes`, etc.).
#[derive(Debug, Clone)]
pub struct BufferCaps {
    pub max_staged_bytes: usize,
    pub chunk_target_chars: usize,
    pub preview_chars: usize,
    pub max_chunks: usize,
    pub page_response_max_chars: usize,
}

impl BufferCaps {
    pub fn from_app_config(config: &AppConfig) -> Self {
        let read_limit = (config.num_ctx as f32 * config.vault_read_ratio) as usize;
        let chunk_target_chars = read_limit.max(512);
        let preview_chars = (chunk_target_chars / 2).max(256);
        let max_staged_bytes = config
            .web_fetch_max_bytes
            .min(chunk_target_chars.saturating_mul(6))
            .max(chunk_target_chars);
        let page_response_max_chars = read_limit.saturating_mul(4).max(4096);
        Self {
            max_staged_bytes,
            chunk_target_chars,
            preview_chars,
            max_chunks: config.ephemeral_buffer_max_chunks.max(1),
            page_response_max_chars,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BufferReceipt {
    pub buffer_id: String,
    pub source: String,
    pub chunk_count: usize,
    pub char_estimate: usize,
    pub preview_head: String,
    pub ttl_secs: u64,
    pub default_page_size: usize,
    pub page_count: usize,
    pub next_step_hint: String,
}

/// Chunk bounded text and insert into ephemeral. `source` is stored in JSON as `url` (web URL or vault path).
pub async fn stage_text(
    ephemeral: &crate::memory::ephemeral::EphemeralMemory,
    tool_name_for_fault: &str,
    source: &str,
    text: &str,
    mut extra_tags: Vec<String>,
    ttl_secs: u64,
    caps: &BufferCaps,
) -> Result<(CacheValue, BufferReceipt)> {
    let (chunks, preview_head) = bound_chunks_and_preview(
        text,
        caps.max_staged_bytes,
        caps.chunk_target_chars,
        caps.preview_chars,
    );
    if chunks.is_empty() {
        return Err(FcpError::ToolFault {
            tool_name: tool_name_for_fault.to_string(),
            reason: "No chunkable content after applying size bounds.".to_string(),
        });
    }
    if chunks.len() > caps.max_chunks {
        return Err(FcpError::ToolFault {
            tool_name: tool_name_for_fault.to_string(),
            reason: format!(
                "Content exceeds ephemeral_buffer_max_chunks ({} > {}). Re-read with a smaller file, raise the cap in config, or split the source.",
                chunks.len(),
                caps.max_chunks
            ),
        });
    }
    let char_estimate: usize = chunks.iter().map(|c| c.len()).sum();
    let blob = BufferedBlob {
        source: source.to_string(),
        chunks: chunks.clone(),
    };
    let payload = serde_json::to_string(&blob).map_err(FcpError::ParseFault)?;
    let mut tags = vec![TAG_EPHEMERAL_BUFFER.to_string()];
    tags.append(&mut extra_tags);
    let title = format!("buffer:{}", uuid::Uuid::new_v4());
    let cache_val = ephemeral
        .insert(&title, &payload, tags, ttl_secs)
        .await?;
    let default_page_size = 1usize;
    let page_count = chunks
        .len()
        .saturating_add(default_page_size - 1)
        / default_page_size;
    let receipt = BufferReceipt {
        buffer_id: cache_val.staged_id.clone(),
        source: source.to_string(),
        chunk_count: chunks.len(),
        char_estimate,
        preview_head,
        ttl_secs,
        default_page_size,
        page_count,
        next_step_hint: "Use ephemeral:buffer_page with the buffer_id from this receipt (short handle such as buf_1), page (0-based), and optional page_size. Use ephemeral:buffer_query with the same buffer_id for keyword search.".to_string(),
    };
    Ok((cache_val, receipt))
}

pub fn parse_buffered_blob(data: &str) -> Result<BufferedBlob> {
    serde_json::from_str(data).map_err(FcpError::ParseFault)
}

/// Whether this entry may be paged or queried as a chunked buffer (web or vault).
pub fn is_chunked_buffer_entry(tags: &[String]) -> bool {
    tags.iter().any(|t| {
        t == TAG_EPHEMERAL_BUFFER || t == "web_artifact" || t == TAG_VAULT_READ_BUFFER
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ChunkLine {
    pub index: usize,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BufferPageResponse {
    pub buffer_id: String,
    pub source: String,
    pub page: usize,
    pub page_size: usize,
    pub page_count: usize,
    pub total_chunks: usize,
    /// Next `page` argument for `ephemeral:buffer_page` when more windows exist (`None` if this was the last page).
    pub next_page: Option<usize>,
    pub chunks: Vec<ChunkLine>,
}

const MAX_PAGE_SIZE_CHUNKS: usize = 64;

/// Slice one page of chunks; enforces `max_response_chars` across returned `text` fields.
pub fn page_chunks(
    tool_name: &str,
    blob: &BufferedBlob,
    buffer_id: &str,
    page: usize,
    page_size: usize,
    max_response_chars: usize,
) -> Result<BufferPageResponse> {
    let page_size = page_size.clamp(1, MAX_PAGE_SIZE_CHUNKS);
    let total = blob.chunks.len();
    if total == 0 {
        return Err(FcpError::ToolFault {
            tool_name: tool_name.to_string(),
            reason: "Buffer has no chunks.".to_string(),
        });
    }
    let page_count = total.saturating_add(page_size - 1) / page_size;
    if page >= page_count {
        let max_index = page_count.saturating_sub(1);
        return Err(FcpError::ToolFault {
            tool_name: tool_name.to_string(),
            reason: format!(
                "page out of range: requested page {} but only {} page(s) exist (total_chunks={}, page_size={}). Use page indices 0 through {} inclusive; when page_count is 1, all chunks are on page 0.",
                page, page_count, total, page_size, max_index
            ),
        });
    }
    let start = page * page_size;
    let end = (start + page_size).min(total);
    let mut out = Vec::new();
    let mut used = 0usize;
    for idx in start..end {
        let raw = &blob.chunks[idx];
        let room = max_response_chars.saturating_sub(used);
        if room == 0 {
            break;
        }
        let piece = truncate_char_boundary(raw, room);
        let truncated = piece.len() < raw.len();
        used += piece.len();
        out.push(ChunkLine {
            index: idx,
            text: piece,
        });
        if truncated {
            break;
        }
    }
    let next_page = if page + 1 < page_count {
        Some(page + 1)
    } else {
        None
    };
    Ok(BufferPageResponse {
        buffer_id: buffer_id.to_string(),
        source: blob.source.clone(),
        page,
        page_size,
        page_count,
        total_chunks: total,
        next_page,
        chunks: out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffered_blob_url_roundtrip() {
        let b = BufferedBlob {
            source: "https://a.test".into(),
            chunks: vec!["one".into(), "two".into()],
        };
        let s = serde_json::to_string(&b).expect("serialize");
        assert!(s.contains("\"url\""));
        let back: BufferedBlob = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(back, b);
    }

    #[test]
    fn page_chunks_first_page() {
        let blob = BufferedBlob {
            source: "p".into(),
            chunks: vec!["a".into(), "b".into(), "c".into()],
        };
        let r = page_chunks("t", &blob, "id", 0, 2, 10_000).expect("page");
        assert_eq!(r.page_count, 2);
        assert_eq!(r.chunks.len(), 2);
        assert_eq!(r.chunks[0].index, 0);
        assert_eq!(r.chunks[1].index, 1);
        assert_eq!(r.next_page, Some(1));
        let last = page_chunks("t", &blob, "id", 1, 2, 10_000).expect("page2");
        assert_eq!(last.next_page, None);
    }

    #[test]
    fn page_chunks_out_of_range_is_fault() {
        let blob = BufferedBlob {
            source: "p".into(),
            chunks: vec!["a".into()],
        };
        let e = page_chunks("t", &blob, "id", 3, 1, 10_000).expect_err("oor");
        match e {
            FcpError::ToolFault { reason, .. } => {
                assert!(reason.contains("out of range"));
            }
            _ => panic!("expected ToolFault"),
        }
    }

    #[test]
    fn page_chunks_respects_char_budget() {
        let blob = BufferedBlob {
            source: "p".into(),
            chunks: vec!["aaaa".into(), "bbbb".into()],
        };
        let r = page_chunks("t", &blob, "id", 0, 2, 5).expect("page");
        assert!(
            r.chunks.iter().map(|c| c.text.len()).sum::<usize>() <= 5,
            "should not exceed budget"
        );
    }
}
