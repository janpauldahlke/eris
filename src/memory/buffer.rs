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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_total_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_lens_start_byte: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_lens_raw_end_byte: Option<usize>,
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

/// Per-chunk navigation for markdown (and plain text) staged buffers: byte offsets in the concatenation
/// of all chunk strings (no delimiter), previews, and optional first `#` heading line in that chunk.
#[derive(Debug, Clone, Serialize)]
pub struct ChunkNavEntry {
    pub chunk_index: usize,
    /// Byte offset where this chunk starts in the concatenation of all chunks.
    pub byte_offset_start: usize,
    /// Byte offset immediately after this chunk in that concatenation.
    pub byte_offset_end: usize,
    pub head_preview: String,
    pub tail_preview: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_heading: Option<String>,
}

/// Build [`ChunkNavEntry`] for each chunk (used in receipts and optional tooling).
pub fn build_chunk_navigation(chunks: &[String], preview_len: usize) -> Vec<ChunkNavEntry> {
    let pl = preview_len.clamp(48, 256);
    let mut global = 0usize;
    let mut out = Vec::with_capacity(chunks.len());
    for (i, c) in chunks.iter().enumerate() {
        let byte_start = global;
        let len = c.len();
        let byte_end = byte_start.saturating_add(len);
        global = byte_end;

        let head_preview = truncate_char_boundary(c, pl);
        let tail_preview = if c.len() > pl {
            let mut skip = c.len().saturating_sub(pl);
            while skip < c.len() && !c.is_char_boundary(skip) {
                skip += 1;
            }
            truncate_char_boundary(&c[skip..], pl)
        } else {
            String::new()
        };

        let first_heading = c
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with('#'))
            .map(|h| truncate_char_boundary(h, 240));

        out.push(ChunkNavEntry {
            chunk_index: i,
            byte_offset_start: byte_start,
            byte_offset_end: byte_end,
            head_preview,
            tail_preview,
            first_heading,
        });
    }
    out
}

/// Vault-only: where this staged buffer sits inside the original file (byte offsets).
#[derive(Debug, Clone, Serialize)]
pub struct VaultLensReceipt {
    pub source_total_bytes: usize,
    /// `[aligned_utf8_start, raw_read_end)` in file byte space; next window typically starts at `[1]`.
    pub lens_file_byte_range: [usize; 2],
    pub suggested_prev_byte_offset: Option<usize>,
    pub suggested_next_byte_offset: Option<usize>,
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
    /// How paging maps to storage: one logical page spans `page_size` consecutive chunk indices.
    pub paging_note: String,
    pub chunk_navigation: Vec<ChunkNavEntry>,
    pub next_step_hint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vault_lens: Option<VaultLensReceipt>,
}

fn build_buffer_receipt(
    buffer_id_fill: &str,
    source: &str,
    chunks: &[String],
    preview_head: String,
    ttl_secs: u64,
    caps: &BufferCaps,
    vault_lens: Option<VaultLensReceipt>,
) -> BufferReceipt {
    let default_page_size = 1usize;
    let page_count = chunks
        .len()
        .saturating_add(default_page_size - 1)
        / default_page_size;
    let chunk_navigation = build_chunk_navigation(chunks, caps.preview_chars.min(200).max(64));
    let paging_note = format!(
        "Paging is over chunk indices: page p with page_size k covers chunk indices [p*k .. min((p+1)*k, {})). Prefer page_size=1 to walk chunk-by-chunk ({} pages). buffer_query returns match-centered excerpts and chunk_index.",
        chunks.len(),
        page_count
    );
    let base_next = "Use ephemeral:buffer_page with buffer_id, page 0..page_count-1, default page_size=1 (one chunk per page). Use ephemeral:buffer_query for keywords; snippets are centered on the first match inside each chunk. If page_count is 1 with a large page_size, all chunks are on page 0—reduce page_size to paginate.".to_string();
    let next_step_hint = if vault_lens.is_some() {
        format!(
            "{} For large vault files, slide the read lens along the file **without changing buffer_id**: call `vault:read` with the same `relative_path`, pass `buffer_id` from this receipt, and set `byte_offset` to `suggested_next_byte_offset` or `suggested_prev_byte_offset` in `vault_lens`.",
            base_next
        )
    } else {
        base_next
    };
    let char_estimate: usize = chunks.iter().map(|c| c.len()).sum();
    BufferReceipt {
        buffer_id: buffer_id_fill.to_string(),
        source: source.to_string(),
        chunk_count: chunks.len(),
        char_estimate,
        preview_head,
        ttl_secs,
        default_page_size,
        page_count,
        paging_note,
        chunk_navigation,
        next_step_hint,
        vault_lens,
    }
}

fn chunked_blob_for_stage(
    source: &str,
    chunks: &[String],
    vault_lens: Option<&VaultLensReceipt>,
) -> BufferedBlob {
    let (vault_total_bytes, vault_lens_start_byte, vault_lens_raw_end_byte) =
        match vault_lens {
            Some(l) => (
                Some(l.source_total_bytes),
                Some(l.lens_file_byte_range[0]),
                Some(l.lens_file_byte_range[1]),
            ),
            None => (None, None, None),
        };
    BufferedBlob {
        source: source.to_string(),
        chunks: chunks.to_vec(),
        vault_total_bytes,
        vault_lens_start_byte,
        vault_lens_raw_end_byte,
    }
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
    vault_lens: Option<VaultLensReceipt>,
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
    let vl_ref = vault_lens.as_ref();
    let blob = chunked_blob_for_stage(source, &chunks, vl_ref);
    let payload = serde_json::to_string(&blob).map_err(FcpError::ParseFault)?;
    let mut tags = vec![TAG_EPHEMERAL_BUFFER.to_string()];
    tags.append(&mut extra_tags);
    let title = format!("buffer:{}", uuid::Uuid::new_v4());
    let cache_val = ephemeral
        .insert(&title, &payload, tags, ttl_secs)
        .await?;
    let receipt = build_buffer_receipt(
        &cache_val.staged_id,
        source,
        &chunks,
        preview_head,
        ttl_secs,
        caps,
        vault_lens,
    );
    Ok((cache_val, receipt))
}

/// Replace an existing staged buffer payload (same `staged_id`) after re-chunking text.
pub async fn stage_text_replace(
    ephemeral: &crate::memory::ephemeral::EphemeralMemory,
    tool_name_for_fault: &str,
    source: &str,
    text: &str,
    extra_tags: Vec<String>,
    ttl_secs: u64,
    caps: &BufferCaps,
    reuse_staged_id: &str,
    vault_lens: Option<VaultLensReceipt>,
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
    let vl_ref = vault_lens.as_ref();
    let blob = chunked_blob_for_stage(source, &chunks, vl_ref);
    let payload = serde_json::to_string(&blob).map_err(FcpError::ParseFault)?;
    let _ = extra_tags;
    let cache_val = ephemeral
        .replace_entry_payload(tool_name_for_fault, reuse_staged_id, &payload, ttl_secs)
        .await?;
    let receipt = build_buffer_receipt(
        reuse_staged_id,
        source,
        &chunks,
        preview_head,
        ttl_secs,
        caps,
        vault_lens,
    );
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
    /// Chunk indices this page request covers (may exceed returned `chunks` if response hit char budget).
    pub chunk_indices_in_page: Vec<usize>,
    /// Chunk indices not yet reached on later pages (empty if `next_page` is null).
    pub remaining_chunk_indices: Vec<usize>,
    /// One-line reminder of paging semantics.
    pub navigation_hint: String,
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
    let chunk_indices_in_page: Vec<usize> = (start..end).collect();
    let remaining_chunk_indices: Vec<usize> = (end..total).collect();
    let navigation_hint = if page_count <= 1 {
        format!(
            "All {total} chunk(s) fit on page 0 at this page_size; there is no page 1. Use a smaller page_size to paginate, or use buffer_query to jump by keyword."
        )
    } else {
        format!(
            "Page index {} of {} pages (valid indices 0 through {}); this window covers chunk indices {} through {} inclusive; {} chunk index(es) remain for later pages.",
            page,
            page_count,
            page_count.saturating_sub(1),
            start,
            end.saturating_sub(1),
            remaining_chunk_indices.len()
        )
    };

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
        chunk_indices_in_page,
        remaining_chunk_indices,
        navigation_hint,
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
            vault_total_bytes: None,
            vault_lens_start_byte: None,
            vault_lens_raw_end_byte: None,
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
            vault_total_bytes: None,
            vault_lens_start_byte: None,
            vault_lens_raw_end_byte: None,
        };
        let r = page_chunks("t", &blob, "id", 0, 2, 10_000).expect("page");
        assert_eq!(r.page_count, 2);
        assert_eq!(r.chunks.len(), 2);
        assert_eq!(r.chunks[0].index, 0);
        assert_eq!(r.chunks[1].index, 1);
        assert_eq!(r.next_page, Some(1));
        let last = page_chunks("t", &blob, "id", 1, 2, 10_000).expect("page2");
        assert_eq!(last.next_page, None);
        assert_eq!(r.chunk_indices_in_page, vec![0, 1]);
        assert_eq!(r.remaining_chunk_indices, vec![2]);
        assert!(r.navigation_hint.contains("chunk indices"));
    }

    #[test]
    fn page_chunks_single_page_navigation_hint() {
        let blob = BufferedBlob {
            source: "p".into(),
            chunks: vec!["a".into(), "b".into()],
            vault_total_bytes: None,
            vault_lens_start_byte: None,
            vault_lens_raw_end_byte: None,
        };
        let r = page_chunks("t", &blob, "id", 0, 2, 10_000).expect("page");
        assert_eq!(r.page_count, 1);
        assert!(r
            .navigation_hint
            .contains("no page 1"));
    }

    #[test]
    fn build_chunk_navigation_records_heading() {
        let chunks = vec!["intro\n# Title here\nbody".into()];
        let nav = build_chunk_navigation(&chunks, 64);
        assert_eq!(nav.len(), 1);
        assert_eq!(nav[0].chunk_index, 0);
        assert_eq!(nav[0].byte_offset_start, 0);
        assert!(nav[0].first_heading.as_deref().unwrap_or("").contains("Title"));
    }

    #[test]
    fn page_chunks_out_of_range_is_fault() {
        let blob = BufferedBlob {
            source: "p".into(),
            chunks: vec!["a".into()],
            vault_total_bytes: None,
            vault_lens_start_byte: None,
            vault_lens_raw_end_byte: None,
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
            vault_total_bytes: None,
            vault_lens_start_byte: None,
            vault_lens_raw_end_byte: None,
        };
        let r = page_chunks("t", &blob, "id", 0, 2, 5).expect("page");
        assert!(
            r.chunks.iter().map(|c| c.text.len()).sum::<usize>() <= 5,
            "should not exceed budget"
        );
    }
}
