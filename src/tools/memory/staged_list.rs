use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

use crate::executive::error::{FcpError, Result};
use crate::memory::ephemeral::EphemeralMemory;
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema, Default)]
pub struct MemoryStagedListArgs {
    pub include_content_preview: Option<bool>,
}

#[derive(Serialize)]
struct StagedEntryView {
    staged_id: String,
    title: String,
    tags: Vec<String>,
    expires_at: u64,
    content_preview: Option<String>,
}

#[derive(Serialize)]
struct StagedListResponse {
    staged_count: usize,
    entries: Vec<StagedEntryView>,
}

pub struct MemoryStagedListTool {
    pub ephemeral: Arc<EphemeralMemory>,
}

#[async_trait]
impl Tool for MemoryStagedListTool {
    fn name(&self) -> &'static str {
        "memory:staged_list"
    }

    fn description(&self) -> &'static str {
        "Lists currently staged ephemeral memories with staged_id, title, tags, and expiry time."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(MemoryStagedListArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: MemoryStagedListArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let include_preview = args.include_content_preview.unwrap_or(false);

        let entries = self
            .ephemeral
            .list_entries()
            .into_iter()
            .map(|entry| StagedEntryView {
                staged_id: entry.staged_id,
                title: entry.title,
                tags: entry.tags,
                expires_at: entry.expires_at,
                content_preview: include_preview.then(|| entry.data.chars().take(120).collect::<String>()),
            })
            .collect::<Vec<_>>();

        let response = StagedListResponse {
            staged_count: entries.len(),
            entries,
        };

        serde_json::to_string(&response).map_err(|e| FcpError::Config(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_staged_list_returns_entries() {
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let _ = ephemeral
            .insert("profile", "Hagbard likes Rust.", vec!["user".to_string()], 60)
            .await
            .unwrap();

        let tool = MemoryStagedListTool { ephemeral };
        let output = tool.execute(serde_json::json!({})).await.unwrap();
        assert!(output.contains("\"staged_count\":1"));
        assert!(output.contains("\"title\":\"profile\""));
    }
}
