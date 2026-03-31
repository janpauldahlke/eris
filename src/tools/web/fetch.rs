use crate::executive::error::{FcpError, Result};
use crate::tools::traits::Tool;
use async_trait::async_trait;
use schemars::JsonSchema;
use schemars::schema::RootSchema;
use serde::Deserialize;
use serde_json::Value;
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
}

impl WebFetchTool {
    pub fn new(timeout_secs: u64, max_bytes: usize) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .build()
            .unwrap_or_else(|_| Client::new());

        Self { client, max_bytes }
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &'static str { "web:fetch" }
    fn description(&self) -> &'static str { "Fetch a webpage and convert its content to Markdown." }
    fn parameters_schema(&self) -> RootSchema { schemars::schema_for!(WebFetchArgs) }

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

        let converter = HtmlToMarkdown::builder().skip_tags(vec!["script", "style", "nav", "footer"]).build();
        let mut markdown = converter.convert(&html).unwrap_or_else(|_| "Failed to parse HTML".into());

        if markdown.len() > self.max_bytes {
            let mut limit = self.max_bytes;
            while limit > 0 && !markdown.is_char_boundary(limit) {
                limit -= 1;
            }
            markdown.truncate(limit);
            markdown.push_str("\n\n[SYSTEM WARNING: CONTENT TRUNCATED DUE TO LENGTH LIMITS]");
        }

        Ok(markdown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use serde_json::json;

    #[tokio::test]
    async fn test_web_fetch_truncation() {
        let server = MockServer::start().await;
        let large_html = format!("<html><body><p>{}</p></body></html>", "A".repeat(1000));
        
        Mock::given(method("GET"))
            .and(path("/large"))
            .respond_with(ResponseTemplate::new(200).set_body_string(large_html))
            .mount(&server)
            .await;

        let tool = WebFetchTool::new(5, 50); // Small byte limit
        let args = json!({ "url": format!("{}/large", server.uri()) });

        let result = tool.execute(args).await.expect("Execution failed");
        assert!(result.contains("[SYSTEM WARNING: CONTENT TRUNCATED DUE TO LENGTH LIMITS]"));
        assert!(result.len() <= 50 + 60); // Limit + Warning length
    }

    #[tokio::test]
    async fn test_web_fetch_404_routing() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/missing"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let tool = WebFetchTool::new(5, 20480);
        let args = json!({ "url": format!("{}/missing", server.uri()) });

        let result = tool.execute(args).await.expect("Execution failed");
        assert_eq!(result, "HTTP Error 404: Not Found");
    }
    
    #[tokio::test]
    async fn test_schema_violation_malformed_url() {
        let tool = WebFetchTool::new(5, 20480);
        let args = json!({ "url": "not-a-link" });

        let result = tool.execute(args).await;
        assert!(matches!(result, Err(FcpError::SchemaViolation(_))));
    }
}