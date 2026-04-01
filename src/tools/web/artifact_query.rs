use crate::executive::error::{FcpError, Result};
use crate::memory::ephemeral::EphemeralMemory;
use crate::tools::traits::Tool;
use async_trait::async_trait;
use schemars::schema::RootSchema;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

#[derive(Deserialize, JsonSchema)]
pub struct WebArtifactQueryArgs {
    pub artifact_id: String,
    pub query: String,
    pub top_k: Option<usize>,
}

#[derive(Serialize, Deserialize)]
struct WebArtifact {
    url: String,
    chunks: Vec<String>,
}

#[derive(Serialize)]
struct ArtifactMatch {
    chunk_index: usize,
    score: usize,
    snippet: String,
}

#[derive(Serialize)]
struct ArtifactQueryResponse {
    artifact_id: String,
    url: String,
    matches: Vec<ArtifactMatch>,
}

pub struct WebArtifactQueryTool {
    pub ephemeral: Arc<EphemeralMemory>,
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

fn trim_chars(input: &str, max_len: usize) -> String {
    if input.len() <= max_len {
        return input.to_string();
    }
    let mut limit = max_len;
    while limit > 0 && !input.is_char_boundary(limit) {
        limit -= 1;
    }
    input[..limit].to_string()
}

fn trim_response_budget(matches: &mut [ArtifactMatch], max_total_chars: usize) {
    let mut used = 0usize;
    for m in matches {
        if used >= max_total_chars {
            m.snippet.clear();
            continue;
        }
        let remaining = max_total_chars - used;
        let trimmed = trim_chars(&m.snippet, remaining);
        used += trimmed.len();
        m.snippet = trimmed;
    }
}

#[async_trait]
impl Tool for WebArtifactQueryTool {
    fn name(&self) -> &'static str {
        "web:artifact_query"
    }

    fn description(&self) -> &'static str {
        "Query sanitized buffered web artifact and return top-k snippets."
    }

    fn parameters_schema(&self) -> RootSchema {
        schemars::schema_for!(WebArtifactQueryArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: WebArtifactQueryArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        if args.artifact_id.trim().is_empty() {
            return Err(FcpError::SchemaViolation("artifact_id cannot be empty".to_string()));
        }

        let entry = self
            .ephemeral
            .get_by_id(&args.artifact_id)
            .await
            .ok_or_else(|| FcpError::ToolFault {
                tool_name: self.name().to_string(),
                reason: "Artifact not found or expired".to_string(),
            })?;
        let artifact: WebArtifact = serde_json::from_str(&entry.data).map_err(FcpError::ParseFault)?;
        let top_k = args.top_k.unwrap_or(3).clamp(1, 3);
        let tokens = tokenize(&args.query);

        let mut scored: Vec<(usize, usize)> = artifact
            .chunks
            .iter()
            .enumerate()
            .map(|(idx, c)| (idx, score_chunk(c, &tokens)))
            .collect();
        scored.sort_by(|a, b| b.1.cmp(&a.1));

        let mut matches = scored
            .into_iter()
            .take(top_k)
            .filter_map(|(idx, score)| {
                artifact.chunks.get(idx).map(|chunk| ArtifactMatch {
                    chunk_index: idx,
                    score,
                    snippet: trim_chars(chunk, self.max_snippet_chars),
                })
            })
            .collect::<Vec<_>>();
        trim_response_budget(&mut matches, self.max_total_chars.max(512));

        let response = ArtifactQueryResponse {
            artifact_id: args.artifact_id,
            url: artifact.url,
            matches,
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
        };
        let payload = serde_json::to_string(&artifact).expect("serialize");
        let stored = mem
            .insert("web_artifact:test", &payload, vec!["web_artifact".into()], 60)
            .await
            .expect("insert");

        let tool = WebArtifactQueryTool {
            ephemeral: mem,
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
    }
}
