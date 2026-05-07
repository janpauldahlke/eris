use crate::executive::error::{FcpError, Result};
use crate::memory::ephemeral::EphemeralMemory;
use crate::memory::semantic::SemanticBrain;
use crate::tools::context_view_hint::{API_TOOL_SNIPPET_CHARS, ToolContextViewHint};
use crate::tools::traits::Tool;
use crate::tools::web::fetch_inner::{
    WebFetchReceiptJson, WebFetchRunOutcome, WebFetchRuntime, build_web_fetch_client,
    default_next_step_hint, run_web_fetch,
};
use async_trait::async_trait;
use reqwest::Client;
use schemars::JsonSchema;
use schemars::schema::RootSchema;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

#[derive(Deserialize, JsonSchema)]
pub struct WebFetchArgs {
    /// The fully qualified URL to fetch (must include https:// or http://)
    pub url: String,
    /// Optional Referer URL (`https://…`). Some origins return 403 to bare navigation; set to the site homepage or listing page you came from.
    #[serde(default)]
    pub referer: Option<String>,
}

pub struct WebFetchTool {
    client: Client,
    max_bytes: usize,
    chunk_chars: usize,
    preview_chars: usize,
    artifact_ttl_secs: u64,
    /// From [`crate::config::AppConfig::web_fetch_default_referer`]; used when the tool args omit `referer`.
    default_referer: Option<String>,
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
        user_agent: String,
        default_referer: Option<String>,
        ephemeral: Arc<EphemeralMemory>,
        semantic: Option<Arc<SemanticBrain>>,
    ) -> Self {
        let client = build_web_fetch_client(timeout_secs, &user_agent);

        Self {
            client,
            max_bytes,
            chunk_chars,
            preview_chars,
            artifact_ttl_secs,
            default_referer,
            ephemeral,
            semantic,
        }
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &'static str {
        "web:fetch"
    }
    fn description(&self) -> &'static str {
        "Fetch webpage, sanitize/chunk externally, and return a compact artifact receipt with heuristic outbound link hints (HTML anchors; filters obvious image/asset URLs)."
    }
    fn parameters_schema(&self) -> RootSchema {
        schemars::schema_for!(WebFetchArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet {
            max_chars: API_TOOL_SNIPPET_CHARS,
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed: WebFetchArgs = match serde_json::from_value(args) {
            Ok(p) => p,
            Err(_) => {
                return Err(FcpError::SchemaViolation(
                    "Invalid arguments for web:fetch".into(),
                ));
            }
        };

        if !parsed.url.starts_with("http://") && !parsed.url.starts_with("https://") {
            return Err(FcpError::SchemaViolation(
                "URL must start with http:// or https://".into(),
            ));
        }

        let rt = WebFetchRuntime {
            client: self.client.clone(),
            max_bytes: self.max_bytes,
            chunk_chars: self.chunk_chars,
            preview_chars: self.preview_chars,
            artifact_ttl_secs: self.artifact_ttl_secs,
            default_referer: self.default_referer.clone(),
            ephemeral: Arc::clone(&self.ephemeral),
            semantic: self.semantic.clone(),
        };

        match run_web_fetch(&rt, parsed.url, parsed.referer).await? {
            WebFetchRunOutcome::Plain(msg) => Ok(msg),
            WebFetchRunOutcome::Stored(stored) => {
                let receipt = WebFetchReceiptJson {
                    artifact_id: stored.artifact_id,
                    url: stored.url,
                    chunk_count: stored.chunk_count,
                    preview_head: stored.preview_head,
                    outbound_links: stored.outbound_links,
                    next_step_hint: default_next_step_hint(),
                };
                serde_json::to_string(&receipt).map_err(FcpError::ParseFault)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::ephemeral::EphemeralMemory;
    use crate::tools::web::fetch_inner::FALLBACK_WEB_FETCH_UA;
    use serde_json::json;
    use std::sync::Arc;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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
            50,
            32,
            32,
            60,
            FALLBACK_WEB_FETCH_UA.to_string(),
            None,
            ephemeral,
            None,
        );
        let args = json!({ "url": format!("{}/large", server.uri()) });

        let result = tool.execute(args).await.expect("Execution failed");
        let parsed: serde_json::Value = serde_json::from_str(&result).expect("receipt json");
        assert!(parsed.get("artifact_id").is_some());
        assert!(parsed.get("preview_head").is_some());
        assert!(parsed.get("outbound_links").is_some());
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
            20480,
            1024,
            256,
            60,
            FALLBACK_WEB_FETCH_UA.to_string(),
            None,
            ephemeral,
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
            20480,
            1024,
            256,
            60,
            FALLBACK_WEB_FETCH_UA.to_string(),
            None,
            ephemeral,
            None,
        );
        let args = json!({ "url": "not-a-link" });

        let result = tool.execute(args).await;
        assert!(matches!(result, Err(FcpError::SchemaViolation(_))));
    }
}
