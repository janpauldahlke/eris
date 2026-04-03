use crate::executive::error::{FcpError, Result};
use crate::ingest::bound_chunks_and_preview;
use crate::memory::ephemeral::EphemeralMemory;
use crate::memory::semantic::SemanticBrain;
use crate::tools::context_view_hint::{ToolContextViewHint, API_TOOL_SNIPPET_CHARS};
use crate::tools::traits::Tool;
use async_trait::async_trait;
use schemars::JsonSchema;
use schemars::schema::RootSchema;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use reqwest::Client;
use htmd::HtmlToMarkdown;

#[derive(Deserialize, JsonSchema)]
pub struct WebFetchArgs {
    /// The fully qualified URL to fetch (must include https:// or http://)
    pub url: String,
}

pub struct WebFetchTool {
    client: Client,
    max_bytes: usize,
    chunk_chars: usize,
    preview_chars: usize,
    artifact_ttl_secs: u64,
    ephemeral: Arc<EphemeralMemory>,
    semantic: Option<Arc<SemanticBrain>>,
}

impl WebFetchTool {
    pub fn new(
        timeout_secs: u64,
        max_bytes: usize,
        chunk_chars: usize,
        preview_chars: usize,
        artifact_ttl_secs: u64,
        ephemeral: Arc<EphemeralMemory>,
        semantic: Option<Arc<SemanticBrain>>,
    ) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            max_bytes,
            chunk_chars,
            preview_chars,
            artifact_ttl_secs,
            ephemeral,
            semantic,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct WebArtifact {
    url: String,
    chunks: Vec<String>,
}

#[derive(Serialize)]
struct WebFetchReceipt {
    artifact_id: String,
    url: String,
    chunk_count: usize,
    preview_head: String,
    next_step_hint: String,
}

fn sanitize_markdown_noise(markdown: &str) -> String {
    let mut out = Vec::new();
    for line in markdown.lines() {
        let l = line.trim();
        if l.is_empty() {
            out.push(String::new());
            continue;
        }
        let ll = l.to_lowercase();
        if ll.contains("cookie settings")
            || ll.contains("accept all cookies")
            || ll.contains("consent preferences")
            || ll.contains("subscribe")
            || ll.contains("newsletter")
            || ll.contains("advertisement")
            || ll.contains("sponsored")
            || ll.contains("privacy policy")
            || ll.contains("terms of service")
        {
            continue;
        }
        out.push(line.to_string());
    }
    out.join("\n")
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &'static str { "web:fetch" }
    fn description(&self) -> &'static str { "Fetch webpage, sanitize/chunk externally, and return a compact artifact receipt." }
    fn parameters_schema(&self) -> RootSchema { schemars::schema_for!(WebFetchArgs) }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet {
            max_chars: API_TOOL_SNIPPET_CHARS,
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed: WebFetchArgs = match serde_json::from_value(args) {
            Ok(p) => p,
            Err(_) => return Err(FcpError::SchemaViolation("Invalid arguments for web:fetch".into())),
        };

        if !parsed.url.starts_with("http://") && !parsed.url.starts_with("https://") {
            return Err(FcpError::SchemaViolation("URL must start with http:// or https://".into()));
        }

        let response = match self.client.get(&parsed.url).send().await {
            Ok(r) => r,
            Err(e) => return Ok(format!("Network Error: {}", e)),
        };

        if !response.status().is_success() {
            return Ok(format!("HTTP Error {}: {}", response.status().as_u16(), response.status().canonical_reason().unwrap_or("Unknown")));
        }

        let html = match response.text().await {
            Ok(t) => t,
            Err(e) => return Ok(format!("Error reading response body: {}", e)),
        };

        let converter = HtmlToMarkdown::builder()
            .skip_tags(vec![
                "script", "style", "nav", "footer", "noscript", "aside", "form", "svg", "header",
            ])
            .build();
        let markdown = converter.convert(&html).unwrap_or_else(|_| "Failed to parse HTML".into());
        let sanitized = sanitize_markdown_noise(&markdown);
        let (chunks, preview_head) = bound_chunks_and_preview(
            &sanitized,
            self.max_bytes,
            self.chunk_chars,
            self.preview_chars,
        );

        if chunks.is_empty() {
            return Ok("No meaningful content extracted from URL.".to_string());
        }

        let artifact = WebArtifact {
            url: parsed.url.clone(),
            chunks: chunks.clone(),
        };
        let serialized = serde_json::to_string(&artifact).map_err(FcpError::ParseFault)?;
        let title = format!("web_artifact:{}", uuid::Uuid::new_v4());
        let stored = self
            .ephemeral
            .insert(
                &title,
                &serialized,
                vec!["web_artifact".to_string(), "external".to_string()],
                self.artifact_ttl_secs,
            )
            .await?;

        if let Some(semantic) = &self.semantic {
            for (chunk_index, chunk) in chunks.iter().enumerate() {
                if let Err(e) = semantic
                    .upsert_web_chunk(&stored.staged_id, &parsed.url, chunk_index, chunk)
                    .await
                {
                    tracing::warn!(
                        artifact_id = %stored.staged_id,
                        chunk_index,
                        error = %e,
                        "Failed to index web artifact chunk; lexical fallback remains available"
                    );
                }
            }
        }

        let receipt = WebFetchReceipt {
            artifact_id: stored.staged_id,
            url: parsed.url,
            chunk_count: chunks.len(),
            preview_head,
            next_step_hint: "Use web:artifact_query with artifact_id and query for targeted retrieval.".to_string(),
        };
        serde_json::to_string(&receipt).map_err(FcpError::ParseFault)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::ephemeral::EphemeralMemory;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use serde_json::json;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_web_fetch_truncation() {
        let server = MockServer::start().await;
        let large_html = format!("<html><body><p>{}</p></body></html>", "A".repeat(1000));
        
        Mock::given(method("GET"))
            .and(path("/large"))
            .respond_with(ResponseTemplate::new(200).set_body_string(large_html))
            .mount(&server)
            .await;

        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let tool = WebFetchTool::new(5, 50, 32, 32, 60, ephemeral, None);
        let args = json!({ "url": format!("{}/large", server.uri()) });

        let result = tool.execute(args).await.expect("Execution failed");
        let parsed: serde_json::Value = serde_json::from_str(&result).expect("receipt json");
        assert!(parsed.get("artifact_id").is_some());
        assert!(parsed.get("preview_head").is_some());
    }

    #[tokio::test]
    async fn test_web_fetch_404_routing() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/missing"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let tool = WebFetchTool::new(5, 20480, 1024, 256, 60, ephemeral, None);
        let args = json!({ "url": format!("{}/missing", server.uri()) });

        let result = tool.execute(args).await.expect("Execution failed");
        assert_eq!(result, "HTTP Error 404: Not Found");
    }
    
    #[tokio::test]
    async fn test_schema_violation_malformed_url() {
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let tool = WebFetchTool::new(5, 20480, 1024, 256, 60, ephemeral, None);
        let args = json!({ "url": "not-a-link" });

        let result = tool.execute(args).await;
        assert!(matches!(result, Err(FcpError::SchemaViolation(_))));
    }
}