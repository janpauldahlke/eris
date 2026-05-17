use crate::executive::error::{FcpError, Result};
use crate::tools::context_view_hint::{API_TOOL_SNIPPET_CHARS, ToolContextViewHint};
use crate::tools::traits::Tool;
use crate::tools::web::context::WebToolContext;
use crate::tools::web::fetch_inner::{WebFetchArgs, WebFetchRunOutcome, run_vault_web_fetch};
use async_trait::async_trait;
use schemars::schema::RootSchema;
use serde_json::Value;

pub struct WebFetchTool {
    pub ctx: WebToolContext,
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &'static str {
        "web:fetch"
    }

    fn description(&self) -> &'static str {
        "Fetch one URL into vault web mission cache (browser39). Returns artifact_id, budget remaining, and up to 3 internal links. Use web:find before re-fetching the same host."
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
        let parsed: WebFetchArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        match run_vault_web_fetch(&self.ctx, parsed).await? {
            WebFetchRunOutcome::Plain(msg) => Ok(msg),
            WebFetchRunOutcome::Stored(stored) => Ok(stored.receipt_json),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WebConfig;
    use crate::tools::web::context::WebFetcherKind;
    use crate::tools::web::fetcher::MockWebFetcher;
    use crate::tools::web::WebSessionLedger;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn test_ctx(dir: &tempfile::TempDir) -> WebToolContext {
        let allowlist_path = dir.path().join(".fcp/web_allowlist.toml");
        std::fs::create_dir_all(allowlist_path.parent().expect("parent")).expect("mkdir");
        std::fs::write(
            &allowlist_path,
            r#"patterns = ["https://example.com/**"]"#,
        )
        .expect("write allowlist");
        WebToolContext {
            vault_root: dir.path().to_path_buf(),
            web: WebConfig::default(),
            web_fetch_user_agent: "test".into(),
            num_ctx: 8192,
            vault_read_ratio: 0.5,
            web_fetch_max_bytes: 20480,
            web_allowlist_override: None,
            ledger: Arc::new(Mutex::new(WebSessionLedger::new())),
            fetcher: WebFetcherKind::Mock(Arc::new(MockWebFetcher::example_com())),
        }
    }

    #[tokio::test]
    async fn fetch_writes_mission_page() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tool = WebFetchTool {
            ctx: test_ctx(&dir),
        };
        let out = tool
            .execute(serde_json::json!({
                "url": "https://example.com/product",
                "mission_note": "Find product X price",
                "fetch_budget": 2
            }))
            .await
            .expect("fetch");
        let receipt: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert!(receipt.get("artifact_id").is_some());
        assert_eq!(receipt.get("cached").and_then(|v| v.as_bool()), Some(false));
    }
}
