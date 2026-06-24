use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::executive::error::{FcpError, Result};
use crate::memory::document_store::{DocumentStore, format_query_results_markdown};
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct DocQueryArgs {
    pub query: String,
    #[serde(default)]
    pub top_k: Option<u32>,
    #[serde(default)]
    pub doc_id: Option<String>,
    #[serde(default)]
    pub min_score: Option<f32>,
    #[serde(default)]
    pub max_total_chars: Option<u32>,
}

pub struct DocQueryTool {
    pub store: Arc<DocumentStore>,
    pub default_top_k: u32,
    pub top_k_max: u32,
    pub default_max_total_chars: u32,
    pub default_min_score: f32,
}

#[async_trait]
impl Tool for DocQueryTool {
    fn name(&self) -> &'static str {
        "doc:query"
    }

    fn description(&self) -> &'static str {
        "Semantic search over ingested document chunks. Returns ranked passages with source citations. Optional doc_id scopes to one document discovered via memory:query or doc:list."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(DocQueryArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed: DocQueryArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        if parsed.query.trim().is_empty() {
            return Err(FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "query is required".into(),
            });
        }

        let top_k = parsed
            .top_k
            .unwrap_or(self.default_top_k)
            .clamp(1, self.top_k_max.max(1));
        let max_total_chars = parsed
            .max_total_chars
            .unwrap_or(self.default_max_total_chars)
            .max(256) as usize;
        let min_score = parsed
            .min_score
            .filter(|s| s.is_finite())
            .map(|s| s.clamp(0.0, 1.0))
            .or(Some(self.default_min_score));

        let chunks = self
            .store
            .query(
                parsed.query.trim(),
                top_k,
                parsed.doc_id.as_deref(),
                min_score,
                max_total_chars,
            )
            .await?;

        Ok(format_query_results_markdown(&chunks))
    }
}
