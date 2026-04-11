use crate::executive::error::{FcpError, Result};
use crate::memory::buffer::{parse_buffered_blob, stage_text, BufferCaps, ChunkNavEntry};
use crate::memory::buffer_handles::BufferHandleRegistry;
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
    caps: BufferCaps,
    buffer_ttl_secs: u64,
    ephemeral: Arc<EphemeralMemory>,
    buffer_handles: Arc<BufferHandleRegistry>,
    semantic: Option<Arc<SemanticBrain>>,
}

impl WebFetchTool {
    pub fn new(
        timeout_secs: u64,
        caps: BufferCaps,
        buffer_ttl_secs: u64,
        ephemeral: Arc<EphemeralMemory>,
        buffer_handles: Arc<BufferHandleRegistry>,
        semantic: Option<Arc<SemanticBrain>>,
    ) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            caps,
            buffer_ttl_secs,
            ephemeral,
            buffer_handles,
            semantic,
        }
    }
}

#[derive(Serialize)]
struct WebFetchReceipt {
    artifact_id: String,
    url: String,
    chunk_count: usize,
    char_estimate: usize,
    preview_head: String,
    ttl_secs: u64,
    default_page_size: usize,
    page_count: usize,
    paging_note: String,
    chunk_navigation: Vec<ChunkNavEntry>,
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
        let (stored, staged_meta) = match stage_text(
            self.ephemeral.as_ref(),
            self.name(),
            &parsed.url,
            &sanitized,
            vec!["web_artifact".to_string(), "external".to_string()],
            self.buffer_ttl_secs,
            &self.caps,
            None,
        )
        .await
        {
            Ok(v) => v,
            Err(FcpError::ToolFault { reason, .. }) if reason.contains("No chunkable content") => {
                return Ok("No meaningful content extracted from URL.".to_string());
            }
            Err(e) => return Err(e),
        };

        let blob = parse_buffered_blob(&stored.data)?;

        let handle = self
            .buffer_handles
            .register(stored.staged_id.clone())
            .await;

        if let Some(semantic) = &self.semantic {
            for (chunk_index, chunk) in blob.chunks.iter().enumerate() {
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
            artifact_id: handle,
            url: parsed.url,
            chunk_count: staged_meta.chunk_count,
            char_estimate: staged_meta.char_estimate,
            preview_head: staged_meta.preview_head,
            ttl_secs: staged_meta.ttl_secs,
            default_page_size: staged_meta.default_page_size,
            page_count: staged_meta.page_count,
            paging_note: staged_meta.paging_note.clone(),
            chunk_navigation: staged_meta.chunk_navigation.clone(),
            next_step_hint: staged_meta.next_step_hint.clone(),
        };
        serde_json::to_string(&receipt).map_err(FcpError::ParseFault)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::buffer::BufferCaps;
    use crate::memory::buffer_handles::BufferHandleRegistry;
    use crate::memory::ephemeral::EphemeralMemory;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use serde_json::json;
    use std::sync::Arc;

    fn caps_small() -> BufferCaps {
        BufferCaps {
            max_staged_bytes: 50,
            chunk_target_chars: 32,
            preview_chars: 32,
            max_chunks: 4096,
            page_response_max_chars: 10_000,
        }
    }

    fn caps_large() -> BufferCaps {
        BufferCaps {
            max_staged_bytes: 20480,
            chunk_target_chars: 1024,
            preview_chars: 256,
            max_chunks: 4096,
            page_response_max_chars: 10_000,
        }
    }

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
        let tool = WebFetchTool::new(
            5,
            caps_small(),
            60,
            ephemeral,
            Arc::new(BufferHandleRegistry::new()),
            None,
        );
        let args = json!({ "url": format!("{}/large", server.uri()) });

        let result = tool.execute(args).await.expect("Execution failed");
        let parsed: serde_json::Value = serde_json::from_str(&result).expect("receipt json");
        let aid = parsed["artifact_id"].as_str().expect("artifact_id");
        assert!(aid.starts_with("buf_"), "expected short handle, got {aid}");
        assert!(parsed.get("preview_head").is_some());
        assert!(parsed.get("chunk_navigation").is_some());
        assert!(parsed["chunk_navigation"].is_array());
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
        let tool = WebFetchTool::new(
            5,
            caps_large(),
            60,
            ephemeral,
            Arc::new(BufferHandleRegistry::new()),
            None,
        );
        let args = json!({ "url": format!("{}/missing", server.uri()) });

        let result = tool.execute(args).await.expect("Execution failed");
        assert_eq!(result, "HTTP Error 404: Not Found");
    }
    
    #[tokio::test]
    async fn test_schema_violation_malformed_url() {
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let tool = WebFetchTool::new(
            5,
            caps_large(),
            60,
            ephemeral,
            Arc::new(BufferHandleRegistry::new()),
            None,
        );
        let args = json!({ "url": "not-a-link" });

        let result = tool.execute(args).await;
        assert!(matches!(result, Err(FcpError::SchemaViolation(_))));
    }
}