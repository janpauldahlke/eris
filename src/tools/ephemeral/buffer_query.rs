use std::sync::Arc;

use async_trait::async_trait;
use schemars::schema::RootSchema;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::executive::error::{FcpError, Result};
use crate::ingest::{trim_chars, trim_snippets_to_budget};
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
    pub query: String,
    pub top_k: Option<usize>,
}

#[derive(Serialize)]
struct BufferMatch {
    chunk_index: usize,
    score: f32,
    snippet: String,
}

#[derive(Serialize)]
struct BufferQueryResponse {
    buffer_id: String,
    /// Source path or URL from the staged blob (same field as `BufferedBlob` JSON `url`).
    #[serde(rename = "url")]
    source: String,
    matches: Vec<BufferMatch>,
}

pub struct EphemeralBufferQueryTool {
    pub ephemeral: Arc<EphemeralMemory>,
    pub buffer_handles: Arc<BufferHandleRegistry>,
    pub semantic: Option<Arc<SemanticBrain>>,
    pub max_snippet_chars: usize,
    pub max_total_chars: usize,
}

fn tokenize(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|s| s.to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

fn score_chunk(chunk: &str, tokens: &[String]) -> usize {
    let c = chunk.to_lowercase();
    tokens.iter().map(|t| usize::from(c.contains(t))).sum()
}

fn lexical_matches(
    chunks: &[String],
    query: &str,
    top_k: usize,
    max_snippet_chars: usize,
) -> Vec<BufferMatch> {
    let tokens = tokenize(query);
    let mut scored: Vec<(usize, usize)> = chunks
        .iter()
        .enumerate()
        .map(|(idx, c)| (idx, score_chunk(c, &tokens)))
        .collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1));

    scored
        .into_iter()
        .take(top_k)
        .filter_map(|(idx, score)| {
            chunks.get(idx).map(|chunk| BufferMatch {
                chunk_index: idx,
                score: score as f32,
                snippet: trim_chars(chunk, max_snippet_chars),
            })
        })
        .collect::<Vec<_>>()
}

#[async_trait]
impl Tool for EphemeralBufferQueryTool {
    fn name(&self) -> &'static str {
        "ephemeral:buffer_query"
    }

    fn description(&self) -> &'static str {
        "Search inside a staged chunked buffer (large vault:read or web:fetch) by buffer_id. Use with ephemeral:buffer_page when you need keyword or semantic hits instead of linear paging; do not invent body text from headings alone."
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
        let top_k = args.top_k.unwrap_or(3).clamp(1, 3);
        let display_id = args.buffer_id.trim().to_string();

        let mut matches = if let Some(semantic) = &self.semantic {
            match semantic
                .search_web_artifact(&args.query, &staged_key, top_k)
                .await
            {
                Ok(semantic_hits) if !semantic_hits.is_empty() => semantic_hits
                    .into_iter()
                    .map(|hit| BufferMatch {
                        chunk_index: hit.chunk_index,
                        score: hit.score,
                        snippet: trim_chars(&hit.snippet, self.max_snippet_chars),
                    })
                    .collect::<Vec<_>>(),
                Ok(_) => lexical_matches(&blob.chunks, &args.query, top_k, self.max_snippet_chars),
                Err(e) => {
                    tracing::warn!(error = %e, "Semantic buffer query failed; using lexical fallback");
                    lexical_matches(&blob.chunks, &args.query, top_k, self.max_snippet_chars)
                }
            }
        } else {
            lexical_matches(&blob.chunks, &args.query, top_k, self.max_snippet_chars)
        };
        let mut snippets = matches
            .iter()
            .map(|m| m.snippet.clone())
            .collect::<Vec<_>>();
        trim_snippets_to_budget(&mut snippets, self.max_total_chars.max(512));
        for (m, snippet) in matches.iter_mut().zip(snippets.into_iter()) {
            m.snippet = snippet;
        }

        let response = BufferQueryResponse {
            buffer_id: display_id,
            source: blob.source,
            matches,
        };
        serde_json::to_string(&response).map_err(FcpError::ParseFault)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let idx = parsed["matches"][0]["chunk_index"].as_u64().unwrap_or(99);
        assert_eq!(idx, 1);
    }
}
