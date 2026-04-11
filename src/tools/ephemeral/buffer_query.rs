use std::sync::Arc;

use async_trait::async_trait;
use schemars::schema::RootSchema;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::executive::error::{FcpError, Result};
use crate::ingest::{trim_snippets_to_budget, truncate_char_boundary};
use crate::memory::buffer::BufferedBlob;
use crate::memory::buffer_handles::{BufferHandleRegistry, BufferHandleResolveError};
use crate::memory::ephemeral::EphemeralMemory;
use crate::memory::semantic::SemanticBrain;
use crate::tools::context_view_hint::{ToolContextViewHint, API_TOOL_SNIPPET_CHARS};
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct EphemeralBufferQueryArgs {
    /// Short handle from the receipt (e.g. `buf_1`), legacy raw UUID, or `artifact_id` alias from web:fetch JSON.
    #[serde(alias = "artifact_id")]
    pub buffer_id: String,
    /// Space-separated keywords per branch; ` or ` (spaces around or) splits alternatives (boolean OR for lexical scoring). Not SQL.
    pub query: String,
    /// Number of hits to return (clamped 1–10; capped by chunk count).
    pub top_k: Option<usize>,
}

#[derive(Serialize)]
struct BufferMatch {
    chunk_index: usize,
    score: f32,
    /// UTF-8 byte offset of the start of the first matched query token in this chunk, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    match_byte_offset_in_chunk: Option<usize>,
    snippet: String,
}

#[derive(Serialize)]
struct BufferQueryResponse {
    buffer_id: String,
    /// Source path or URL from the staged blob (same field as `BufferedBlob` JSON `url`).
    #[serde(rename = "url")]
    source: String,
    /// How the server interpreted the query for lexical scoring (OR branches, tokenization).
    query_note: String,
    match_count: usize,
    matches: Vec<BufferMatch>,
}

pub struct EphemeralBufferQueryTool {
    pub ephemeral: Arc<EphemeralMemory>,
    pub buffer_handles: Arc<BufferHandleRegistry>,
    pub semantic: Option<Arc<SemanticBrain>>,
    pub max_snippet_chars: usize,
    pub max_total_chars: usize,
}

/// Split on ` or ` (ASCII, case-insensitive) for lexical OR branches.
fn split_or_alternatives(query: &str) -> Vec<String> {
    let q = query.trim();
    if q.is_empty() {
        return Vec::new();
    }
    let lower = q.to_lowercase();
    let mut out = Vec::new();
    let mut start = 0usize;
    while let Some(rel) = lower[start..].find(" or ") {
        let abs = start + rel;
        let part = q[start..abs].trim();
        if !part.is_empty() {
            out.push(part.to_string());
        }
        start = abs + 4;
    }
    let last = q[start..].trim();
    if !last.is_empty() {
        out.push(last.to_string());
    }
    if out.is_empty() {
        out.push(q.to_string());
    }
    out
}

fn is_lexical_stopword(s: &str) -> bool {
    matches!(
        s,
        "or" | "and" | "the" | "a" | "an" | "to" | "of" | "in" | "for" | "on"
    )
}

fn normalize_token(raw: &str) -> Option<String> {
    let t = raw
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '’' || c == '`');
    if t.is_empty() {
        return None;
    }
    let t = t.trim_matches(|c: char| {
        !c.is_alphanumeric() && c != '-' && c != '_' && c != '#'
    });
    if t.is_empty() {
        return None;
    }
    let low = t.to_lowercase();
    if is_lexical_stopword(&low) {
        return None;
    }
    Some(low)
}

fn tokenize_alternative(fragment: &str) -> Vec<String> {
    fragment
        .split_whitespace()
        .filter_map(|w| normalize_token(w))
        .collect()
}

struct LexicalQueryPlan {
    alt_tokens: Vec<Vec<String>>,
    query_note: String,
}

fn plan_lexical_query(raw: &str) -> LexicalQueryPlan {
    let alternatives = split_or_alternatives(raw);
    let alt_tokens: Vec<Vec<String>> = alternatives
        .iter()
        .map(|a| tokenize_alternative(a))
        .filter(|t| !t.is_empty())
        .collect();

    let query_note = if alt_tokens.is_empty() {
        "No searchable tokens after normalizing the query (try keywords without only stopwords).".to_string()
    } else if alt_tokens.len() == 1 {
        format!(
            "Lexical AND-of-tokens within one phrase: {:?}. Not SQL or regex.",
            alt_tokens[0]
        )
    } else {
        format!(
            "Lexical OR across {} alternative phrase(s) (split on ' or '); each phrase is AND-of-tokens. Example tokens per branch: {:?}",
            alt_tokens.len(),
            alt_tokens
                .iter()
                .map(|v| v.as_slice())
                .collect::<Vec<_>>()
        )
    };

    LexicalQueryPlan {
        alt_tokens,
        query_note,
    }
}

fn score_chunk_lower(lower: &str, tokens: &[String]) -> usize {
    tokens.iter().map(|t| usize::from(lower.contains(t))).sum()
}

fn find_earliest_token_byte_offset(lower_chunk: &str, tokens: &[String]) -> Option<usize> {
    let mut best: Option<usize> = None;
    for t in tokens {
        if t.is_empty() {
            continue;
        }
        if let Some(pos) = lower_chunk.find(t.as_str()) {
            best = Some(best.map_or(pos, |b| b.min(pos)));
        }
    }
    best
}

/// Build a bounded excerpt around a UTF-8 byte offset (ellipsis when clipped).
fn excerpt_around_byte_offset(chunk: &str, byte_off: usize, max_len: usize) -> String {
    if chunk.is_empty() {
        return String::new();
    }
    if chunk.len() <= max_len {
        return chunk.to_string();
    }
    let mut start = byte_off.saturating_sub(max_len / 2);
    while start > 0 && !chunk.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (start + max_len).min(chunk.len());
    while end > start && !chunk.is_char_boundary(end) {
        end -= 1;
    }
    if end.saturating_sub(start) < max_len.saturating_mul(3) / 4 {
        start = byte_off.saturating_sub(max_len.saturating_mul(3) / 4);
        while start > 0 && !chunk.is_char_boundary(start) {
            start -= 1;
        }
        end = (start + max_len).min(chunk.len());
        while end > start && !chunk.is_char_boundary(end) {
            end -= 1;
        }
    }
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.push_str(&chunk[start..end]);
    if end < chunk.len() {
        out.push('…');
    }
    out
}

fn lexical_matches(
    chunks: &[String],
    plan: &LexicalQueryPlan,
    top_k: usize,
    max_snippet_chars: usize,
) -> Vec<BufferMatch> {
    if plan.alt_tokens.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(usize, usize, usize)> = Vec::new();
    for (idx, c) in chunks.iter().enumerate() {
        let lower = c.to_lowercase();
        let mut best_score = 0usize;
        let mut best_alt = 0usize;
        for (ai, tokens) in plan.alt_tokens.iter().enumerate() {
            let s = score_chunk_lower(&lower, tokens);
            if s > best_score {
                best_score = s;
                best_alt = ai;
            }
        }
        if best_score > 0 {
            scored.push((idx, best_score, best_alt));
        }
    }
    scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    scored
        .into_iter()
        .take(top_k)
        .filter_map(|(idx, score, best_alt)| {
            chunks.get(idx).map(|chunk| {
                let tokens = &plan.alt_tokens[best_alt];
                let lower = chunk.to_lowercase();
                let off = find_earliest_token_byte_offset(&lower, tokens);
                let (snippet, match_off) = match off {
                    None => (
                        truncate_char_boundary(chunk, max_snippet_chars),
                        None,
                    ),
                    Some(b) => (
                        excerpt_around_byte_offset(chunk, b, max_snippet_chars),
                        Some(b),
                    ),
                };
                BufferMatch {
                    chunk_index: idx,
                    score: score as f32,
                    match_byte_offset_in_chunk: match_off,
                    snippet,
                }
            })
        })
        .collect::<Vec<_>>()
}

fn union_tokens_for_refinement(plan: &LexicalQueryPlan) -> Vec<String> {
    let mut seen = std::collections::HashSet::<String>::new();
    let mut out = Vec::new();
    for t in plan.alt_tokens.iter().flatten() {
        if seen.insert(t.clone()) {
            out.push(t.clone());
        }
    }
    out
}

#[async_trait]
impl Tool for EphemeralBufferQueryTool {
    fn name(&self) -> &'static str {
        "ephemeral:buffer_query"
    }

    fn description(&self) -> &'static str {
        "Search inside a staged chunked buffer (large vault:read or web:fetch) by buffer_id. Query is lexical: space-separated keywords; use ` or ` between alternatives (e.g. `HITL or human loop`). Not SQL. Pair with ephemeral:buffer_page when hits are only front matter/TOC and you need later chunks."
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet {
            max_chars: API_TOOL_SNIPPET_CHARS,
        }
    }

    fn parameters_schema(&self) -> RootSchema {
        schemars::schema_for!(EphemeralBufferQueryArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: EphemeralBufferQueryArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        if args.buffer_id.trim().is_empty() {
            return Err(FcpError::SchemaViolation(
                "buffer_id cannot be empty".to_string(),
            ));
        }

        let staged_key = match self
            .buffer_handles
            .resolve_for_lookup(&args.buffer_id)
            .await
        {
            Ok(k) => k,
            Err(BufferHandleResolveError::Empty) => {
                return Err(FcpError::SchemaViolation(
                    "buffer_id cannot be empty".to_string(),
                ));
            }
            Err(BufferHandleResolveError::UnknownHandle) => {
                return Err(FcpError::ToolFault {
                    tool_name: self.name().to_string(),
                    reason: "Unknown buffer_id; use the buf_N token from your latest vault:read or web:fetch receipt or the [FCP BUFFER SESSION] block.".to_string(),
                });
            }
        };

        let entry = self
            .ephemeral
            .get_by_id(&staged_key)
            .await
            .ok_or_else(|| FcpError::ToolFault {
                tool_name: self.name().to_string(),
                reason: "Buffer not found or expired; re-stage with vault:read or web:fetch.".to_string(),
            })?;
        let blob: BufferedBlob = serde_json::from_str(&entry.data).map_err(FcpError::ParseFault)?;
        let top_k = args
            .top_k
            .unwrap_or(3)
            .clamp(1, 10)
            .min(blob.chunks.len().max(1));
        let display_id = args.buffer_id.trim().to_string();
        let plan = plan_lexical_query(&args.query);
        let query_tokens = union_tokens_for_refinement(&plan);

        let mut matches = if let Some(semantic) = &self.semantic {
            match semantic
                .search_web_artifact(&args.query, &staged_key, top_k)
                .await
            {
                Ok(semantic_hits) if !semantic_hits.is_empty() => semantic_hits
                    .into_iter()
                    .map(|hit| {
                        let full_chunk = blob.chunks.get(hit.chunk_index);
                        let (snippet, match_off) = if !query_tokens.is_empty() {
                            if let Some(ch) = full_chunk {
                                let lower = ch.to_lowercase();
                                if let Some(b) =
                                    find_earliest_token_byte_offset(&lower, &query_tokens)
                                {
                                    (
                                        excerpt_around_byte_offset(
                                            ch,
                                            b,
                                            self.max_snippet_chars,
                                        ),
                                        Some(b),
                                    )
                                } else {
                                    (
                                        truncate_char_boundary(
                                            &hit.snippet,
                                            self.max_snippet_chars,
                                        ),
                                        None,
                                    )
                                }
                            } else {
                                (
                                    truncate_char_boundary(
                                        &hit.snippet,
                                        self.max_snippet_chars,
                                    ),
                                    None,
                                )
                            }
                        } else {
                            (
                                truncate_char_boundary(&hit.snippet, self.max_snippet_chars),
                                None,
                            )
                        };
                        BufferMatch {
                            chunk_index: hit.chunk_index,
                            score: hit.score,
                            match_byte_offset_in_chunk: match_off,
                            snippet,
                        }
                    })
                    .collect::<Vec<_>>(),
                Ok(_) => lexical_matches(
                    &blob.chunks,
                    &plan,
                    top_k,
                    self.max_snippet_chars,
                ),
                Err(e) => {
                    tracing::warn!(error = %e, "Semantic buffer query failed; using lexical fallback");
                    lexical_matches(
                        &blob.chunks,
                        &plan,
                        top_k,
                        self.max_snippet_chars,
                    )
                }
            }
        } else {
            lexical_matches(
                &blob.chunks,
                &plan,
                top_k,
                self.max_snippet_chars,
            )
        };
        let mut snippets = matches
            .iter()
            .map(|m| m.snippet.clone())
            .collect::<Vec<_>>();
        trim_snippets_to_budget(&mut snippets, self.max_total_chars.max(512));
        for (m, snippet) in matches.iter_mut().zip(snippets.into_iter()) {
            m.snippet = snippet;
        }

        let match_count = matches.len();
        let response = BufferQueryResponse {
            buffer_id: display_id,
            source: blob.source,
            query_note: plan.query_note,
            match_count,
            matches,
        };
        serde_json::to_string(&response).map_err(FcpError::ParseFault)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn or_alternative_matches_either_branch() {
        let plan = plan_lexical_query("foo OR bar");
        assert!(plan.alt_tokens.len() >= 2);
        let chunks = vec![
            "only foo here".to_string(),
            "only bar here".to_string(),
        ];
        let m = lexical_matches(&chunks, &plan, 4, 200);
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn quoted_tokens_normalize() {
        let t = tokenize_alternative("\"human\"");
        assert!(t.contains(&"human".to_string()));
    }

    #[tokio::test]
    async fn buffer_query_returns_ranked_snippets() {
        let mem = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let blob = BufferedBlob {
            source: "https://example.com".to_string(),
            chunks: vec![
                "sports and weather".to_string(),
                "economy and markets update".to_string(),
                "international politics".to_string(),
            ],
            vault_total_bytes: None,
            vault_lens_start_byte: None,
            vault_lens_raw_end_byte: None,
        };
        let payload = serde_json::to_string(&blob).expect("serialize");
        let stored = mem
            .insert("web_artifact:test", &payload, vec!["web_artifact".into()], 60)
            .await
            .expect("insert");

        let tool = EphemeralBufferQueryTool {
            ephemeral: mem,
            buffer_handles: Arc::new(crate::memory::buffer_handles::BufferHandleRegistry::new()),
            semantic: None,
            max_snippet_chars: 64,
            max_total_chars: 256,
        };
        let res = tool
            .execute(serde_json::json!({
                "buffer_id": stored.staged_id,
                "query": "economy markets",
                "top_k": 1
            }))
            .await
            .expect("query");
        let parsed: serde_json::Value = serde_json::from_str(&res).expect("json");
        assert!(parsed["query_note"].is_string());
        assert_eq!(parsed["match_count"], 1);
        let idx = parsed["matches"][0]["chunk_index"].as_u64().unwrap_or(99);
        assert_eq!(idx, 1);
        let off = parsed["matches"][0]["match_byte_offset_in_chunk"]
            .as_u64()
            .expect("match offset");
        assert_eq!(off, 0, "economy is at start of chunk 1");
    }
}
