use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::executive::error::{FcpError, Result};
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct MemoryQueryArgs {
    pub query: String,
    pub filter_tag: Option<String>,
    pub file_path: Option<String>,
}

pub struct MemoryQueryTool {
    pub workspace: String,
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
        let _args: MemoryQueryArgs = serde_json::from_value(args)
            .map_err(|e| FcpError::ParseFault(e))?;

        // Structural stub: Fails correctly to satisfy TDD cycle
        Err(FcpError::ToolFault {
            tool_name: self.name().into(),
            reason: "Not implemented: Requires Qdrant and OllamaClient integration".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_memory_query_execution() {
        let tool = MemoryQueryTool {
            workspace: "test_workspace".into(),
        };
        let args = serde_json::json!({
            "query": "find auth logic",
            "filter_tag": "security",
            "file_path": "src/auth.rs"
        });

        let result = tool.execute(args).await;
        
        assert!(result.is_err());
        if let Err(crate::executive::error::FcpError::ToolFault { reason, .. }) = result {
            assert!(reason.contains("Not implemented"));
        } else {
            panic!("Expected ToolFault for unimplemented tool");
        }
    }
}
