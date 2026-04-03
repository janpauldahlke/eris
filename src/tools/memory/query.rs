use std::sync::Arc;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::executive::error::{FcpError, Result};
use crate::tools::traits::Tool;
use crate::memory::semantic::{MemoryQueryOptions, SemanticBrain};

#[derive(Deserialize, JsonSchema)]
pub struct MemoryQueryArgs {
    pub query: String,
    /// Optional. Only use when you know an exact tag string from indexed vault metadata (e.g. `user`).
    /// Wrong tags return no filtered hits; the tool may fall back to global search and prefix a WARNING.
    pub filter_tag: Option<String>,
    /// Max number of hits to return after ranking; clamped to `memory_query_top_k_max` in config. Default `memory_query_default_top_k`.
    #[serde(default)]
    pub top_k: Option<u32>,
    /// Total character budget for the formatted result. Default `memory_query_default_max_total_chars` in config.
    #[serde(default)]
    pub max_total_chars: Option<u32>,
    /// Drop hits with cosine similarity score below this threshold (0.0–1.0). Omit for no floor.
    #[serde(default)]
    pub min_score: Option<f32>,
    /// Only include points whose indexed `vault_key` starts with this prefix (e.g. `30_Persons/`).
    #[serde(default)]
    pub vault_path_prefix: Option<String>,
}

pub struct MemoryQueryTool {
    pub workspace: String,
    pub semantic: Arc<SemanticBrain>,
    pub default_top_k: u32,
    pub top_k_max: u32,
    pub default_max_total_chars: u32,
    pub min_max_total_chars: u32,
    pub qdrant_oversample_cap: u64,
    pub qdrant_oversample_multiplier: u64,
    pub qdrant_oversample_min: u64,
}

#[async_trait]
impl Tool for MemoryQueryTool {
    fn name(&self) -> &'static str {
        "memory:query"
    }

    fn description(&self) -> &'static str {
        "Search long-term semantic memory. Prefer `query` alone first; omit `filter_tag` unless you know the exact tag from vault frontmatter (e.g. user, about_me). Optional: `top_k`, `max_total_chars`, `min_score`, `vault_path_prefix` (e.g. `30_Persons/`). Defaults and caps come from `.fcp/config.toml` (`memory_query_*`). A wrong `filter_tag` may yield a WARNING and unfiltered fallback. For an exact file path, use vault:read."
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

        let top_k_max = self.top_k_max.max(1);
        let default_top_k = self.default_top_k.max(1).min(top_k_max);

        let top_k = args
            .top_k
            .unwrap_or(default_top_k)
            .clamp(1, top_k_max) as u64;

        let min_floor = self.min_max_total_chars.max(1);
        let max_total_chars = args
            .max_total_chars
            .unwrap_or(self.default_max_total_chars)
            .max(min_floor) as usize;

        let min_score = args.min_score.filter(|s| s.is_finite()).map(|s| s.clamp(0.0, 1.0));

        let vault_path_prefix = args
            .vault_path_prefix
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty());

        let options = MemoryQueryOptions {
            top_k,
            filter_tag: filter,
            vault_path_prefix,
            min_score,
            max_total_chars,
            qdrant_oversample_cap: self.qdrant_oversample_cap,
            qdrant_oversample_multiplier: self.qdrant_oversample_multiplier,
            qdrant_oversample_min: self.qdrant_oversample_min,
        };

        let outcome = self
            .semantic
            .search_memory_query(&args.query, options)
            .await?;

        let mut text = String::new();
        if outcome.used_fallback {
            if let Some(tag) = outcome.attempted_filter_tag.as_deref() {
                text.push_str(&format!(
                    "[WARNING: Tag '{tag}' matched no indexed points. Results below are from an unfiltered global search.]\n\n"
                ));
            }
        }
        if outcome.used_vault_prefix_fallback {
            if let Some(prefix) = outcome.attempted_vault_prefix.as_deref() {
                text.push_str(&format!(
                    "[WARNING: vault_path_prefix '{prefix}' matched no points with that path prefix. Results below are from a broader search without path prefix.]\n\n"
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
