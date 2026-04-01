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
    /// When set, only points whose indexed `tags` contain this value are returned (matches upsert payload).
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
        "Search long-term semantic memory. Use this to retrieve facts, remember past conversations, or answer questions about the user's name, preferences, and identity. For reading a specific vault path, use vault:read."
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
        let results = self.semantic.search(&args.query, 5, filter).await?;
        
        if results.is_empty() {
            Ok(format!("No semantic memory found for query: {}", args.query))
        } else {
            Ok(results)
        }
    }
}

#[cfg(test)]
mod tests {
    // Testing requires SemanticBrain, which needs a live Qdrant instance.
}
