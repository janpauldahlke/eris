//! `web:search` — run a web search via browser39's configured search engine, then vault-cache results.

use crate::executive::error::{FcpError, Result};
use crate::tools::context_view_hint::{API_TOOL_SNIPPET_CHARS, ToolContextViewHint};
use crate::tools::traits::Tool;
use crate::tools::web::allowlist::{enforce_allowlist, load_allowlist};
use crate::tools::web::context::WebToolContext;
use crate::tools::web::fetch_inner::{WebFetchArgs, WebFetchRunOutcome, run_vault_web_fetch};
use crate::vault_layout;
use async_trait::async_trait;
use schemars::JsonSchema;
use schemars::schema::RootSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;

const DEFAULT_SEARCH_ENGINE: &str = "https://html.duckduckgo.com/html/?q={}";

#[derive(Deserialize, JsonSchema)]
pub struct WebSearchArgs {
    /// Plain-language search query (not a URL).
    pub query: String,
    #[serde(default)]
    pub mission_note: Option<String>,
    #[serde(default)]
    pub mission_id: Option<String>,
    #[serde(default)]
    pub fetch_budget: Option<u32>,
}

pub struct WebSearchTool {
    pub ctx: WebToolContext,
}

pub fn load_search_engine_template(vault_root: &Path) -> Result<String> {
    let path = vault_layout::fcp_dir(vault_root).join("browser39/config.toml");
    if !path.is_file() {
        return Ok(DEFAULT_SEARCH_ENGINE.to_string());
    }
    let raw = std::fs::read_to_string(&path).map_err(FcpError::Io)?;
    let table: toml::Value = toml::from_str(&raw).map_err(|e| {
        FcpError::Config(format!(
            "invalid browser39 config {}: {e}",
            path.display()
        ))
    })?;
    table
        .get("search")
        .and_then(|s| s.get("engine"))
        .and_then(|e| e.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            FcpError::Config(format!(
                "browser39 config {} missing [search].engine",
                path.display()
            ))
        })
}

pub fn build_search_url(engine_template: &str, query: &str) -> Result<String> {
    let q = query.trim();
    if q.is_empty() {
        return Err(FcpError::SchemaViolation(
            "web:search: query must be non-empty".into(),
        ));
    }
    let encoded: String = url::form_urlencoded::byte_serialize(q.as_bytes()).collect();
    let template = engine_template.trim();
    if template.contains("{}") {
        Ok(template.replacen("{}", &encoded, 1))
    } else if template.ends_with('=') || template.ends_with('/') {
        Ok(format!("{template}{encoded}"))
    } else {
        Ok(format!("{template}?q={encoded}"))
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &'static str {
        "web:search"
    }

    fn description(&self) -> &'static str {
        "Search the web using the vault browser39 search engine (see `.fcp/browser39/config.toml` [search].engine), then cache the results page like web:fetch. The search provider URL must be on `.fcp/web_allowlist.toml`."
    }

    fn parameters_schema(&self) -> RootSchema {
        schemars::schema_for!(WebSearchArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet {
            max_chars: API_TOOL_SNIPPET_CHARS,
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        if !self.ctx.web.search_enabled {
            return Err(FcpError::PolicyViolation {
                code: "WEB_SEARCH_DISABLED".into(),
                message: "web search is disabled (set [web].search_enabled = true)".into(),
            });
        }
        let args: WebSearchArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let engine = load_search_engine_template(&self.ctx.vault_root)?;
        let url = build_search_url(&engine, &args.query)?;
        let allowlist = load_allowlist(
            &self.ctx.vault_root,
            self.ctx.web_allowlist_override.as_deref(),
        )?;
        enforce_allowlist(self.ctx.web.allowlist_enabled, &url, &allowlist)?;
        let note = args.mission_note.unwrap_or_else(|| {
            format!("web:search: {}", args.query.trim())
        });
        match run_vault_web_fetch(
            &self.ctx,
            WebFetchArgs {
                url,
                mission_note: Some(note),
                mission_id: args.mission_id,
                fetch_budget: args.fetch_budget,
                selector: None,
                explore_site: false,
            },
        )
        .await?
        {
            WebFetchRunOutcome::Plain(msg) => Ok(msg),
            WebFetchRunOutcome::Stored(stored) => Ok(stored.receipt_json),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_search_url_duckduckgo() {
        let url = build_search_url(
            "https://html.duckduckgo.com/html/?q={}",
            "bundesliga letzter spieltag",
        )
        .expect("url");
        assert!(url.contains("bundesliga"));
        assert!(url.starts_with("https://html.duckduckgo.com/"));
    }
}
