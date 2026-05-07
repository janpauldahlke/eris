use crate::executive::error::{FcpError, Result};
use crate::ingest::{trim_chars, trim_snippets_to_budget};
use crate::memory::ephemeral::EphemeralMemory;
use crate::memory::semantic::SemanticBrain;
use crate::tools::context_view_hint::{ARTIFACT_QUERY_SNIPPET_CHARS, ToolContextViewHint};
use crate::tools::traits::Tool;
use crate::tools::web::artifact::{WebArtifact, WebOutboundLink};
use crate::tools::web::markdown_focus::chunk_heading_weight_factor;
use async_trait::async_trait;
use schemars::JsonSchema;
use schemars::schema::RootSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

#[derive(Deserialize, JsonSchema)]
pub struct WebArtifactQueryArgs {
    pub artifact_id: String,
    pub query: String,
    pub top_k: Option<usize>,
}

#[derive(Serialize)]
struct ArtifactMatch {
    chunk_index: usize,
    score: f32,
    snippet: String,
}

/// Ranked `<a href>` hints attached to the query response (capped; full list is on the `web:fetch` receipt).
const ARTIFACT_QUERY_OUTBOUND_CAP: usize = 12;

#[derive(Serialize)]
struct ArtifactQueryResponse {
    artifact_id: String,
    url: String,
    /// Lexical or semantic hits — listed **first** so LLM context truncation still surfaces body text.
    matches: Vec<ArtifactMatch>,
    /// Subset of stored outbound links (same order/ranks as fetch; remainder omitted to save space).
    outbound_links: Vec<WebOutboundLink>,
}

pub struct WebArtifactQueryTool {
    pub ephemeral: Arc<EphemeralMemory>,
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
) -> Vec<ArtifactMatch> {
    let tokens = tokenize(query);
    let mut scored: Vec<(usize, f32)> = chunks
        .iter()
        .enumerate()
        .map(|(idx, c)| {
            let base = score_chunk(c, &tokens) as f32;
            let w = chunk_heading_weight_factor(c);
            (idx, base * w)
        })
        .collect();
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });

    scored
        .into_iter()
        .take(top_k)
        .filter_map(|(idx, score)| {
            chunks.get(idx).map(|chunk| ArtifactMatch {
                chunk_index: idx,
                score,
                snippet: trim_chars(chunk, max_snippet_chars),
            })
        })
        .collect::<Vec<_>>()
}

#[async_trait]
impl Tool for WebArtifactQueryTool {
    fn name(&self) -> &'static str {
        "web:artifact_query"
    }

    fn description(&self) -> &'static str {
        "Query buffered web artifact: returns top-k chunk text matches first, then a capped list of outbound article links (full link list is on the web:fetch receipt)."
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet {
            max_chars: ARTIFACT_QUERY_SNIPPET_CHARS,
        }
    }

    fn parameters_schema(&self) -> RootSchema {
        schemars::schema_for!(WebArtifactQueryArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: WebArtifactQueryArgs =
            serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        if args.artifact_id.trim().is_empty() {
            return Err(FcpError::SchemaViolation(
                "artifact_id cannot be empty".to_string(),
            ));
        }

        let entry = self
            .ephemeral
            .get_by_id(&args.artifact_id)
            .await
            .ok_or_else(|| FcpError::ToolFault {
                tool_name: self.name().to_string(),
                reason: "Artifact not found or expired".to_string(),
            })?;
        let artifact: WebArtifact =
            serde_json::from_str(&entry.data).map_err(FcpError::ParseFault)?;
        let top_k = args.top_k.unwrap_or(3).clamp(1, 3);
        let mut matches = if let Some(semantic) = &self.semantic {
            match semantic
                .search_web_artifact(&args.query, &args.artifact_id, top_k)
                .await
            {
                Ok(semantic_hits) if !semantic_hits.is_empty() => semantic_hits
                    .into_iter()
                    .map(|hit| ArtifactMatch {
                        chunk_index: hit.chunk_index,
                        score: hit.score,
                        snippet: trim_chars(&hit.snippet, self.max_snippet_chars),
                    })
                    .collect::<Vec<_>>(),
                Ok(_) => {
                    lexical_matches(&artifact.chunks, &args.query, top_k, self.max_snippet_chars)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Semantic artifact query failed; using lexical fallback");
                    lexical_matches(&artifact.chunks, &args.query, top_k, self.max_snippet_chars)
                }
            }
        } else {
            lexical_matches(&artifact.chunks, &args.query, top_k, self.max_snippet_chars)
        };
        let mut snippets = matches
            .iter()
            .map(|m| m.snippet.clone())
            .collect::<Vec<_>>();
        trim_snippets_to_budget(&mut snippets, self.max_total_chars.max(512));
        for (m, snippet) in matches.iter_mut().zip(snippets.into_iter()) {
            m.snippet = snippet;
        }

        let outbound_links = artifact
            .outbound_links
            .iter()
            .take(ARTIFACT_QUERY_OUTBOUND_CAP)
            .cloned()
            .collect::<Vec<_>>();
        let response = ArtifactQueryResponse {
            artifact_id: args.artifact_id,
            url: artifact.url,
            matches,
            outbound_links,
        };
        serde_json::to_string(&response).map_err(FcpError::ParseFault)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_artifact_query_returns_ranked_snippets() {
        let mem = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let artifact = WebArtifact {
            url: "https://example.com".to_string(),
            chunks: vec![
                "sports and weather".to_string(),
                "economy and markets update".to_string(),
                "international politics".to_string(),
            ],
            outbound_links: vec![WebOutboundLink {
                url: "https://example.com/deep/article".to_string(),
                anchor_text: Some("Story".to_string()),
                rank: 1,
            }],
        };
        let payload = serde_json::to_string(&artifact).expect("serialize");
        let stored = mem
            .insert(
                "web_artifact:test",
                &payload,
                vec!["web_artifact".into()],
                60,
            )
            .await
            .expect("insert");

        let tool = WebArtifactQueryTool {
            ephemeral: mem,
            semantic: None,
            max_snippet_chars: 64,
            max_total_chars: 256,
        };
        let res = tool
            .execute(serde_json::json!({
                "artifact_id": stored.staged_id,
                "query": "economy markets",
                "top_k": 1
            }))
            .await
            .expect("query");
        let parsed: serde_json::Value = serde_json::from_str(&res).expect("json");
        let idx = parsed["matches"][0]["chunk_index"].as_u64().unwrap_or(99);
        assert_eq!(idx, 1);
        assert_eq!(
            parsed["outbound_links"][0]["url"].as_str().unwrap_or(""),
            "https://example.com/deep/article"
        );
    }
}
