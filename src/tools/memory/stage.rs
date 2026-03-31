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
    pub title: String,
    pub content: String,
    pub tags: Vec<String>,
}

pub struct MemoryStageTool {
    pub ephemeral: Arc<EphemeralMemory>,
    pub ttl_secs: u64,
    pub max_content_chars: usize,
}

#[async_trait]
impl Tool for MemoryStageTool {
    fn name(&self) -> &'static str {
        "memory:stage"
    }

    fn description(&self) -> &'static str {
        "Stages content into ephemeral memory with a title, tags, and TTL. Content auto-promotes to vault on TTL expiry."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(MemoryStageArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: MemoryStageArgs = serde_json::from_value(args)
            .map_err(FcpError::ParseFault)?;

        if args.title.trim().is_empty() {
            return Err(FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "Title cannot be empty".into(),
            });
        }

        if args.content.trim().is_empty() {
            return Err(FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "Content cannot be empty".into(),
            });
        }

        if args.content.len() > self.max_content_chars {
            return Err(FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: format!(
                    "Content exceeds max size ({} chars > {} limit)",
                    args.content.len(),
                    self.max_content_chars,
                ),
            });
        }

        self.ephemeral.insert(&args.title, &args.content, args.tags.clone(), self.ttl_secs).await?;
        Ok(format!(
            "Staged '{}' with tags {:?} (ttl={}s, auto-promotes on expiry)",
            args.title, args.tags, self.ttl_secs
        ))
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
            max_content_chars: 10_000,
        };
        let args = serde_json::json!({
            "title": "infrastructure",
            "content": "The database uses port 5432.",
            "tags": ["db", "postgres"]
        });

        let result = tool.execute(args).await;
        assert!(result.is_ok());
        let entry = ephemeral.get_entry("infrastructure").await.unwrap();
        assert_eq!(entry.data, "The database uses port 5432.");
        assert_eq!(entry.tags, vec!["db", "postgres"]);
    }

    #[tokio::test]
    async fn test_memory_stage_rejects_oversized_content() {
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let tool = MemoryStageTool {
            ephemeral: ephemeral.clone(),
            ttl_secs: 60,
            max_content_chars: 10,
        };
        let args = serde_json::json!({
            "title": "big",
            "content": "This content is way too long for the limit",
            "tags": ["test"]
        });

        let result = tool.execute(args).await;
        assert!(result.is_err());
    }
}
