use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::executive::error::{FcpError, Result};
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema, PartialEq, Debug, Clone)]
#[serde(rename_all = "PascalCase")]
pub enum TargetDomain {
    Semantic,
    Episodic,
}

#[derive(Deserialize, JsonSchema)]
pub struct MemoryCommitArgs {
    pub tag: String,
    pub target_domain: TargetDomain,
}

pub struct MemoryCommitTool {
    pub workspace_root: std::path::PathBuf,
}

#[async_trait]
impl Tool for MemoryCommitTool {
    fn name(&self) -> &'static str {
        "memory:commit"
    }

    fn description(&self) -> &'static str {
        "Pulls moka cache entries for tag, writes them to physical vault, and invalidates keys."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(MemoryCommitArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let _args: MemoryCommitArgs = serde_json::from_value(args)
            .map_err(|e| FcpError::ParseFault(e))?;

        // Structural stub: Fails correctly to satisfy TDD cycle
        Err(FcpError::ToolFault {
            tool_name: self.name().into(),
            reason: "Not implemented: Requires moka read, formatting, physical file write, and cache invalidation".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_memory_commit_execution() {
        let tool = MemoryCommitTool {
            workspace_root: std::path::PathBuf::from("/tmp/test_workspace"),
        };
        let args = serde_json::json!({
            "tag": "infrastructure",
            "target_domain": "Semantic"
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
