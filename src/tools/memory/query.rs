use std::sync::Arc;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::executive::error::{FcpError, Result};
use crate::tools::traits::Tool;
use crate::memory::semantic::SemanticBrain;

#[derive(Deserialize, JsonSchema)]
pub struct MemoryQueryArgs {
    pub query: String,
    /// Optional. Only use when you know an exact tag string from indexed vault metadata (e.g. `user`).
    /// Wrong tags return no filtered hits; the tool may fall back to global search and prefix a WARNING.
    pub filter_tag: Option<String>,
}

pub struct MemoryQueryTool {
    pub workspace: String,
    pub semantic: Arc<SemanticBrain>,
}

#[async_trait]
impl Tool for MemoryQueryTool {
    fn name(&self) -> &'static str {
        "memory:query"
    }

    fn description(&self) -> &'static str {
        "Search long-term semantic memory. Prefer `query` alone first; omit `filter_tag` unless you know the exact tag from vault frontmatter (e.g. user, about_me). A wrong `filter_tag` yields no filtered matches; results may then come from an unfiltered search with a leading WARNING line so you can correct the tag next time. For an exact file path, use vault:read."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(MemoryQueryArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: MemoryQueryArgs = serde_json::from_value(args)
            .map_err(FcpError::ParseFault)?;

        if args.query.trim().is_empty() {
            return Err(FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "Query cannot be empty".to_string(),
            });
        }

        let filter = args
            .filter_tag
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty());

        let outcome = self
            .semantic
            .search_memory_query(&args.query, 5, filter)
            .await?;

        let mut text = String::new();
        if outcome.used_fallback {
            if let Some(tag) = outcome.attempted_filter_tag.as_deref() {
                text.push_str(&format!(
                    "[WARNING: Tag '{tag}' matched no indexed points. Results below are from an unfiltered global search.]\n\n"
                ));
            }
        }
        text.push_str(&outcome.markdown);

        if text.trim().is_empty() {
            Ok(format!("No semantic memory found for query: {}", args.query))
        } else {
            Ok(text)
        }
    }
}

#[cfg(test)]
mod tests {
    // Testing requires SemanticBrain, which needs a live Qdrant instance.
}
