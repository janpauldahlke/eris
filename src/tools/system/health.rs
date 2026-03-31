use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::executive::error::{FcpError, Result};
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct SystemHealthArgs {}

pub struct SystemHealthTool;

#[async_trait]
impl Tool for SystemHealthTool {
    fn name(&self) -> &'static str {
        "system:health"
    }

    fn description(&self) -> &'static str {
        "Pings local Qdrant, reads active moka cache size, and checks workspace lockfile seal. Returns a dense JSON string."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(SystemHealthArgs)
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        // Structural stub: Fails correctly to satisfy TDD cycle
        Err(FcpError::ToolFault {
            tool_name: self.name().into(),
            reason: "Not implemented: Requires Qdrant ping and Moka cache stats".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_system_health_execution() {
        let tool = SystemHealthTool;
        let args = serde_json::json!({});

        let result = tool.execute(args).await;
        
        assert!(result.is_err());
        if let Err(crate::executive::error::FcpError::ToolFault { reason, .. }) = result {
            assert!(reason.contains("Not implemented"));
        } else {
            panic!("Expected ToolFault for unimplemented tool");
        }
    }
}
