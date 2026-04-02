//! English Wikipedia REST `page/summary` via [`crate::util::ApiHttpClient`].

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::util::ApiHttpClient;
use crate::executive::error::{FcpError, Result};
use crate::tools::traits::Tool;

pub const PROFILE_WIKIPEDIA_PAGE_SUMMARY: &str = "wikipedia_page_summary";

pub const HINT_WIKI_SUMMARY: &str =
    "English Wikipedia lead summary only; may be incomplete or dated. Not your vault or arbitrary URLs.";

#[derive(Deserialize, JsonSchema)]
pub struct WikiSummaryArgs {
    /// Wikipedia article title as a human would say it (e.g. "Rust (programming language)", "Paris").
    pub title: String,
}

#[derive(Debug, Deserialize)]
struct WikipediaSummaryBody {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    extract: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    content_urls: Option<ContentUrls>,
}

#[derive(Debug, Deserialize)]
struct ContentUrls {
    #[serde(default)]
    desktop: Option<DesktopPage>,
}

#[derive(Debug, Deserialize)]
struct DesktopPage {
    #[serde(default)]
    page: Option<String>,
}

pub struct WikiSummaryTool {
    pub api: Arc<ApiHttpClient>,
}

pub fn map_api_err(tool_name: &'static str, e: FcpError) -> FcpError {
    match e {
        FcpError::ToolFault { tool_name: tn, reason } if tn == "api_client" => FcpError::ToolFault {
            tool_name: tool_name.to_string(),
            reason,
        },
        FcpError::NetworkFault(_) => FcpError::ToolFault {
            tool_name: tool_name.to_string(),
            reason: "Wikipedia summary service unreachable".into(),
        },
        FcpError::Config(msg) => FcpError::ToolFault {
            tool_name: tool_name.to_string(),
            reason: format!("Wikipedia API configuration: {msg}"),
        },
        other => other,
    }
}

/// Percent-encode a title for use as a single REST path segment.
pub fn encode_title_path_segment(title: &str) -> String {
    utf8_percent_encode(title.trim(), NON_ALPHANUMERIC).to_string()
}

pub async fn run_wiki_summary(api: &ApiHttpClient, title: &str) -> Result<String> {
    let encoded = encode_title_path_segment(title);
    if encoded.is_empty() {
        return Err(FcpError::SchemaViolation(
            "title must be a non-empty string".into(),
        ));
    }
    let mut params = HashMap::new();
    params.insert("title".into(), encoded);
    let body = api
        .get_templated(PROFILE_WIKIPEDIA_PAGE_SUMMARY, &params)
        .await
        .map_err(|e| map_api_err("wiki:summary", e))?;
    let parsed: WikipediaSummaryBody = serde_json::from_str(&body).map_err(|e| {
        FcpError::ToolFault {
            tool_name: "wiki:summary".into(),
            reason: format!("Wikipedia summary JSON parse error: {e}"),
        }
    })?;
    let canonical_url = parsed
        .content_urls
        .as_ref()
        .and_then(|c| c.desktop.as_ref())
        .and_then(|d| d.page.clone());
    let envelope = json!({
        "tool": "wiki:summary",
        "source": "english_wikipedia",
        "hint": HINT_WIKI_SUMMARY,
        "title": parsed.title,
        "description": parsed.description,
        "extract": parsed.extract,
        "canonical_url": canonical_url,
    });
    serde_json::to_string(&envelope).map_err(FcpError::ParseFault)
}

#[async_trait]
impl Tool for WikiSummaryTool {
    fn name(&self) -> &'static str {
        "wiki:summary"
    }

    fn description(&self) -> &'static str {
        "English Wikipedia lead summary by article title (REST page/summary). Use for encyclopedia-style facts (what/who is X). Do not use for pasted URLs or non-Wikipedia sites (use web:fetch). Do not use to search your vault (use vault:read / memory:query)."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(WikiSummaryArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed: WikiSummaryArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let title = parsed.title.trim();
        if title.is_empty() {
            return Err(FcpError::SchemaViolation(
                "title must be a non-empty string".into(),
            ));
        }
        run_wiki_summary(self.api.as_ref(), title).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_title_path_segment_spaces() {
        assert_eq!(
            encode_title_path_segment("Albert Einstein"),
            "Albert%20Einstein"
        );
    }

    #[test]
    fn encode_title_path_segment_trims() {
        assert_eq!(encode_title_path_segment("  Earth  "), "Earth");
    }
}
