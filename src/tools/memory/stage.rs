use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::executive::error::{FcpError, Result};
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct MemoryStageArgs {
    pub content: String,
    pub tag: String,
}

pub struct MemoryStageTool;

#[async_trait]
impl Tool for MemoryStageTool {
    fn name(&self) -> &'static str {
        "memory:stage"
    }

    fn description(&self) -> &'static str {
        "Injects content into the moka cache wrapping it with absolute SystemTime."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(MemoryStageArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let _args: MemoryStageArgs = serde_json::from_value(args)
            .map_err(|e| FcpError::ParseFault(e))?;

        // Structural stub: Fails correctly to satisfy TDD cycle
        Err(FcpError::ToolFault {
            tool_name: self.name().into(),
            reason: "Not implemented: Requires moka cache injection".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_memory_stage_execution() {
        let tool = MemoryStageTool;
        let args = serde_json::json!({
            "content": "The database uses port 5432.",
            "tag": "infrastructure"
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
