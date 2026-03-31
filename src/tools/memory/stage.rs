use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use crate::executive::error::{FcpError, Result};
use crate::memory::ephemeral::EphemeralMemory;
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct MemoryStageArgs {
    pub content: String,
    pub tag: String,
}

pub struct MemoryStageTool {
    pub ephemeral: Arc<EphemeralMemory>,
    pub ttl_secs: u64,
}

#[async_trait]
impl Tool for MemoryStageTool {
    fn name(&self) -> &'static str {
        "memory:stage"
    }

    fn description(&self) -> &'static str {
        "Stages content into ephemeral memory under a tag with TTL."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(MemoryStageArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: MemoryStageArgs = serde_json::from_value(args)
            .map_err(FcpError::ParseFault)?;

        self.ephemeral.insert(&args.tag, &args.content, self.ttl_secs).await?;
        Ok(format!("Staged memory for tag '{}' (ttl={}s)", args.tag, self.ttl_secs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_memory_stage_execution() {
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let tool = MemoryStageTool {
            ephemeral: ephemeral.clone(),
            ttl_secs: 60,
        };
        let args = serde_json::json!({
            "content": "The database uses port 5432.",
            "tag": "infrastructure"
        });

        let result = tool.execute(args).await;
        assert!(result.is_ok());
        let staged = ephemeral.get("infrastructure").await;
        assert_eq!(staged, Some("The database uses port 5432.".to_string()));
    }
}
