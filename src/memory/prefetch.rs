//! Turn-start semantic prefetch: deterministic runtime recall (not an LLM tool).

use std::time::{Duration, Instant};

use crate::config::AppConfig;
use crate::ingest::truncate_char_boundary;
use crate::memory::semantic::{MemoryHit, SemanticBrain};
use crate::orchestrator::tool_router::ToolRouter;

const PREFETCH_HEADER: &str = "[RELEVANT_LEARNED_MEMORY]\n";
const PREFETCH_FOOTER: &str = "[/RELEVANT_LEARNED_MEMORY]\n";

/// Snippet body from indexed embed text (drops Title/Tags header when present).
pub fn snippet_from_indexed_text(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    let body = if let Some(idx) = trimmed.find("\n\n") {
        trimmed[idx + 2..].trim()
    } else {
        trimmed
    };
    let use_body = if body.is_empty() { trimmed } else { body };
    truncate_char_boundary(use_body, max_chars)
}

/// Content-only block for the system prompt (no vault paths or scores).
pub fn format_prefetch_content_only(
    hits: &[MemoryHit],
    max_total_chars: usize,
    max_per_hit: usize,
) -> String {
    if hits.is_empty() || max_total_chars == 0 {
        return String::new();
    }

    let mut lines: Vec<String> = Vec::new();
    let mut used = PREFETCH_HEADER.len() + PREFETCH_FOOTER.len();

    for hit in hits {
        let snippet = snippet_from_indexed_text(&hit.text, max_per_hit);
        if snippet.is_empty() {
            continue;
        }
        let line = if lines.is_empty() {
            snippet
        } else {
            format!("\n{snippet}")
        };
        if used + line.len() > max_total_chars {
            break;
        }
        used += line.len();
        lines.push(line);
    }

    if lines.is_empty() {
        return String::new();
    }

    let mut out = String::with_capacity(used);
    out.push_str(PREFETCH_HEADER);
    for line in lines {
        out.push_str(&line);
    }
    out.push_str(PREFETCH_FOOTER);
    out
}

/// Run semantic prefetch for one user turn. Returns `None` when skipped, empty, timed out, or errored.
pub async fn run_turn_prefetch(
    semantic: &SemanticBrain,
    user_text: &str,
    config: &AppConfig,
) -> Option<String> {
    if !config.memory_prefetch_enabled {
        return None;
    }

    let trimmed = user_text.trim();
    if trimmed.len() < config.memory_prefetch_min_user_chars {
        return None;
    }
    if ToolRouter::short_input_guard_conversational_only(trimmed) {
        return None;
    }

    let timeout = Duration::from_secs(config.memory_prefetch_timeout_secs.max(1));
    let top_k = usize::try_from(config.memory_prefetch_top_k.max(1)).unwrap_or(2);
    let min_score = Some(config.memory_prefetch_min_score.clamp(0.0, 1.0));
    let query = trimmed.to_string();

    let started = Instant::now();
    let search = tokio::time::timeout(
        timeout,
        semantic.semantic_search_hits(&query, top_k, min_score),
    )
    .await;

    match search {
        Ok(Ok(hits)) => {
            let prefetch_ms = started.elapsed().as_millis() as u64;
            for hit in &hits {
                tracing::debug!(
                    target: "fcp.memory_prefetch",
                    vault_key = ?hit.vault_key,
                    score = hit.score,
                    prefetch_ms,
                    "prefetch hit"
                );
            }
            let block = format_prefetch_content_only(
                &hits,
                config.memory_prefetch_max_chars,
                config.memory_prefetch_max_chars_per_hit,
            );
            if block.is_empty() {
                tracing::debug!(
                    target: "fcp.memory_prefetch",
                    prefetch_ms,
                    "prefetch skipped: no hits above threshold"
                );
                return None;
            }
            tracing::debug!(
                target: "fcp.memory_prefetch",
                hit_count = hits.len(),
                chars = block.len(),
                prefetch_ms,
                "prefetch block ready"
            );
            Some(block)
        }
        Ok(Err(e)) => {
            tracing::warn!(
                target: "fcp.memory_prefetch",
                error = %e,
                "prefetch failed; continuing without block"
            );
            None
        }
        Err(_) => {
            tracing::warn!(
                target: "fcp.memory_prefetch",
                timeout_secs = config.memory_prefetch_timeout_secs,
                "prefetch timed out; continuing without block"
            );
            None
        }
    }
}

// ── Document-aware prefetch ──────────────────────────────────────────

const DOC_PREFETCH_HEADER: &str = "[RELEVANT_DOCUMENT_CONTEXT]\n";
const DOC_PREFETCH_FOOTER: &str = "[/RELEVANT_DOCUMENT_CONTEXT]\n";

/// Run document prefetch for one user turn. Returns `None` when disabled,
/// no hits, timed out, or errored.
pub async fn run_document_prefetch(
    doc_store: &crate::memory::document_store::DocumentStore,
    user_text: &str,
    config: &AppConfig,
) -> Option<String> {
    if !config.document_rag.document_prefetch_enabled {
        return None;
    }

    let trimmed = user_text.trim();
    if trimmed.len() < config.memory_prefetch_min_user_chars {
        return None;
    }
    if ToolRouter::short_input_guard_conversational_only(trimmed) {
        return None;
    }

    let timeout = Duration::from_secs(config.document_rag.document_prefetch_timeout_secs.max(1));
    let top_k = config.document_rag.document_prefetch_top_k.max(1);
    let min_score = Some(config.document_rag.document_prefetch_min_score.clamp(0.0, 1.0));
    let max_total = config.document_rag.document_prefetch_max_chars.max(64);
    let max_per_hit = config.document_rag.document_prefetch_max_chars_per_hit.max(64);

    let started = Instant::now();
    let search = tokio::time::timeout(
        timeout,
        doc_store.query(trimmed, top_k, None, min_score, max_total),
    )
    .await;

    match search {
        Ok(Ok(chunks)) => {
            let prefetch_ms = started.elapsed().as_millis() as u64;
            if chunks.is_empty() {
                tracing::debug!(
                    target: "fcp.document_prefetch",
                    prefetch_ms,
                    "document prefetch skipped: no hits above threshold"
                );
                return None;
            }
            let block = format_doc_prefetch_block(&chunks, max_total, max_per_hit);
            if block.is_empty() {
                return None;
            }
            tracing::debug!(
                target: "fcp.document_prefetch",
                hit_count = chunks.len(),
                chars = block.len(),
                prefetch_ms,
                "document prefetch block ready"
            );
            Some(block)
        }
        Ok(Err(e)) => {
            tracing::warn!(
                target: "fcp.document_prefetch",
                error = %e,
                "document prefetch failed; continuing without block"
            );
            None
        }
        Err(_) => {
            tracing::warn!(
                target: "fcp.document_prefetch",
                timeout_secs = config.document_rag.document_prefetch_timeout_secs,
                "document prefetch timed out; continuing without block"
            );
            None
        }
    }
}

fn format_doc_prefetch_block(
    chunks: &[crate::memory::document_store::DocumentChunk],
    max_total_chars: usize,
    max_per_hit: usize,
) -> String {
    if chunks.is_empty() || max_total_chars == 0 {
        return String::new();
    }

    let mut lines: Vec<String> = Vec::new();
    let mut used = DOC_PREFETCH_HEADER.len() + DOC_PREFETCH_FOOTER.len();

    for chunk in chunks {
        let snippet = truncate_char_boundary(chunk.text.trim(), max_per_hit);
        if snippet.is_empty() {
            continue;
        }
        let attribution = format!(
            "(from {}, chunk {}/{})\n{}",
            chunk.source_name,
            chunk.chunk_index,
            chunk.total_chunks,
            snippet
        );
        let line = if lines.is_empty() {
            attribution
        } else {
            format!("\n{attribution}")
        };
        if used + line.len() > max_total_chars && !lines.is_empty() {
            break;
        }
        used += line.len();
        lines.push(line);
    }

    if lines.is_empty() {
        return String::new();
    }

    let mut out = String::with_capacity(used);
    out.push_str(DOC_PREFETCH_HEADER);
    for line in lines {
        out.push_str(&line);
    }
    out.push_str(DOC_PREFETCH_FOOTER);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::semantic::MemoryHit;

    #[test]
    fn snippet_strips_title_tags_header() {
        let text = "Title: Pauline\nTags: contact\n\nEmail: pauline@example.com";
        assert_eq!(
            snippet_from_indexed_text(text, 80),
            "Email: pauline@example.com"
        );
    }

    #[test]
    fn format_prefetch_has_no_vault_key_or_score() {
        let hits = vec![MemoryHit {
            score: 0.61,
            text: "Title: X\n\nPauline email pauline@example.com".into(),
            vault_key: Some("30_Synthesis/abc/r0001.md".into()),
            recency_ts_ms: None,
        }];
        let block = format_prefetch_content_only(&hits, 600, 280);
        assert!(block.contains(PREFETCH_HEADER.trim()));
        assert!(block.contains("pauline@example.com"));
        assert!(!block.contains("30_Synthesis"));
        assert!(!block.contains("0.61"));
    }

    #[test]
    fn format_prefetch_empty_when_no_hits() {
        assert!(format_prefetch_content_only(&[], 600, 280).is_empty());
    }

    #[test]
    fn format_prefetch_respects_total_budget() {
        let hits = vec![
            MemoryHit {
                score: 0.9,
                text: "a".repeat(400),
                vault_key: None,
                recency_ts_ms: None,
            },
            MemoryHit {
                score: 0.8,
                text: "b".repeat(400),
                vault_key: None,
                recency_ts_ms: None,
            },
        ];
        let block = format_prefetch_content_only(&hits, 120, 80);
        assert!(block.len() <= 120);
    }

    #[test]
    fn doc_prefetch_formats_with_attribution() {
        use crate::memory::document_store::DocumentChunk;
        let chunks = vec![DocumentChunk {
            text: "The Transformer follows an encoder-decoder structure.".into(),
            doc_id: "abc".into(),
            source_path: "99_USER_UPLOADED/files/paper.pdf".into(),
            source_name: "attention_paper.pdf".into(),
            chunk_index: 12,
            total_chunks: 87,
            content_hash: "deadbeef".into(),
            ingested_at_ms: 1,
            score: 0.72,
        }];
        let block = format_doc_prefetch_block(&chunks, 800, 350);
        assert!(block.contains(DOC_PREFETCH_HEADER.trim()));
        assert!(block.contains("attention_paper.pdf"));
        assert!(block.contains("chunk 12/87"));
        assert!(block.contains("encoder-decoder"));
    }

    #[test]
    fn doc_prefetch_empty_when_no_chunks() {
        assert!(format_doc_prefetch_block(&[], 800, 350).is_empty());
    }

    #[test]
    fn doc_prefetch_respects_budget() {
        use crate::memory::document_store::DocumentChunk;
        let chunks = vec![
            DocumentChunk {
                text: "a".repeat(400),
                doc_id: "x".into(),
                source_path: "p".into(),
                source_name: "doc.pdf".into(),
                chunk_index: 0,
                total_chunks: 2,
                content_hash: "h".into(),
                ingested_at_ms: 1,
                score: 0.9,
            },
            DocumentChunk {
                text: "b".repeat(400),
                doc_id: "x".into(),
                source_path: "p".into(),
                source_name: "doc.pdf".into(),
                chunk_index: 1,
                total_chunks: 2,
                content_hash: "h".into(),
                ingested_at_ms: 1,
                score: 0.8,
            },
        ];
        let block = format_doc_prefetch_block(&chunks, 150, 80);
        assert!(block.len() <= 250);
    }
}
