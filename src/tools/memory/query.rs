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
    pub filter_tag: Option<String>,
    pub file_path: Option<String>,
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
        "Semantically search the vault (Semantic Zoom). Returns up to 1500 tokens of context."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(MemoryQueryArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: MemoryQueryArgs = serde_json::from_value(args)
            .map_err(|e| FcpError::ParseFault(e))?;

        let results = self.semantic.search(&args.query, 5).await?;
        
        if results.is_empty() {
            Ok(format!("No semantic memory found for query: {}", args.query))
        } else {
            Ok(results)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Testing requires SemanticBrain, which needs network. Testing omitted here to avoid network calls.
}
