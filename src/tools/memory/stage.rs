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
    /// A short descriptive title (e.g. "hagbard_profile", "weather_api"). Required.
    pub title: Option<String>,
    /// The text content to remember. Required.
    pub content: Option<String>,
    /// Taxonomy tags for vault routing. Use: person/contact → 30_Persons, user/preference → 40_User, semantic/knowledge/api → 20_Semantic. Required, at least one tag.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
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

        let title = args.title.filter(|t| !t.trim().is_empty())
            .ok_or_else(|| FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "title is required — provide a short descriptive title (e.g. \"hagbard_profile\", \"weather_api\")".into(),
            })?;

        let content = args.content.filter(|c| !c.trim().is_empty())
            .ok_or_else(|| FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "content is required — provide the actual text to remember".into(),
            })?;

        let tags = args.tags.filter(|t| !t.is_empty())
            .ok_or_else(|| FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "tags is required — provide at least one tag for vault routing (e.g. [\"person\",\"hagbard\"] or [\"api\",\"weather\"])".into(),
            })?;

        if content.len() > self.max_content_chars {
            return Err(FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: format!(
                    "Content exceeds max size ({} chars > {} limit)",
                    content.len(),
                    self.max_content_chars,
                ),
            });
        }

        self.ephemeral.insert(&title, &content, tags.clone(), self.ttl_secs).await?;
        Ok(format!(
            "Staged '{}' with tags {:?} (ttl={}s, auto-promotes on expiry)",
            title, tags, self.ttl_secs
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
    async fn test_memory_stage_rejects_null_title() {
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let tool = MemoryStageTool {
            ephemeral: ephemeral.clone(),
            ttl_secs: 60,
            max_content_chars: 10_000,
        };
        let args = serde_json::json!({
            "title": null,
            "content": "Hagbard is the primary user",
            "tags": ["person"]
        });

        let result = tool.execute(args).await;
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("title is required"));
    }

    #[tokio::test]
    async fn test_memory_stage_rejects_null_tags() {
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let tool = MemoryStageTool {
            ephemeral: ephemeral.clone(),
            ttl_secs: 60,
            max_content_chars: 10_000,
        };
        let args = serde_json::json!({
            "title": "hagbard",
            "content": "Hagbard is the primary user",
            "tags": null
        });

        let result = tool.execute(args).await;
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("tags is required"));
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

    #[tokio::test]
    async fn test_memory_stage_rejects_empty_content() {
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let tool = MemoryStageTool {
            ephemeral: ephemeral.clone(),
            ttl_secs: 60,
            max_content_chars: 10_000,
        };
        let args = serde_json::json!({
            "title": null,
            "content": "",
            "tags": null
        });

        let result = tool.execute(args).await;
        assert!(result.is_err());
    }
}
