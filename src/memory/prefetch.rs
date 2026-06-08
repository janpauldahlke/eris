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
}
