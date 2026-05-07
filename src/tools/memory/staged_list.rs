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
    /// Filter by tier: "session", "scratch", or "promote". Omit for all tiers.
    #[serde(default)]
    pub filter_tier: Option<crate::memory::types::EphemeralTier>,
    /// When true, only show entries with needs_review == true.
    #[serde(default)]
    pub only_needs_review: Option<bool>,
}

#[derive(Serialize)]
struct StagedEntryView {
    staged_id: String,
    title: String,
    tags: Vec<String>,
    expires_at: u64,
    tier: crate::memory::types::EphemeralTier,
    promotion_score: f64,
    needs_review: bool,
    kind: crate::memory::types::VaultKind,
    node_id: String,
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
        "Lists staged ephemeral memories with tier, score, needs_review, kind, node_id. Optional filter_tier and only_needs_review."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(MemoryStagedListArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: MemoryStagedListArgs =
            serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let include_preview = args.include_content_preview.unwrap_or(false);

        let filter_tier = args.filter_tier;
        let only_review = args.only_needs_review.unwrap_or(false);

        let entries = self
            .ephemeral
            .list_entries()
            .into_iter()
            .filter(|e| filter_tier.is_none_or(|t| e.tier == t))
            .filter(|e| !only_review || e.needs_review)
            .map(|entry| StagedEntryView {
                staged_id: entry.staged_id,
                title: entry.title,
                tags: entry.tags,
                expires_at: entry.expires_at,
                tier: entry.tier,
                promotion_score: entry.promotion_score,
                needs_review: entry.needs_review,
                kind: entry.kind,
                node_id: entry.node_id,
                content_preview: include_preview
                    .then(|| entry.data.chars().take(120).collect::<String>()),
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
            .insert(
                "profile",
                "Hagbard likes Rust.",
                vec!["user".to_string()],
                60,
            )
            .await
            .unwrap();

        let tool = MemoryStagedListTool { ephemeral };
        let output = tool.execute(serde_json::json!({})).await.unwrap();
        assert!(output.contains("\"staged_count\":1"));
        assert!(output.contains("\"title\":\"profile\""));
    }

    #[tokio::test]
    async fn test_staged_list_filter_by_tier() {
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        ephemeral
            .insert_with_tier(
                "a",
                "data",
                vec!["t".into()],
                60,
                crate::memory::types::EphemeralTier::Session,
                crate::memory::types::VaultKind::Synthesis,
            )
            .await
            .unwrap();
        ephemeral
            .insert_with_tier(
                "b",
                "data",
                vec!["t".into()],
                60,
                crate::memory::types::EphemeralTier::Promote,
                crate::memory::types::VaultKind::Synthesis,
            )
            .await
            .unwrap();

        let tool = MemoryStagedListTool { ephemeral };
        let output = tool
            .execute(serde_json::json!({"filter_tier": "promote"}))
            .await
            .unwrap();
        assert!(output.contains("\"staged_count\":1"));
        assert!(output.contains("\"title\":\"b\""));
    }

    #[tokio::test]
    async fn test_staged_list_only_needs_review() {
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let entry = ephemeral
            .insert("contested", "data", vec!["t".into()], 60)
            .await
            .unwrap();
        ephemeral.set_needs_review(&entry.staged_id, true).await;
        ephemeral
            .insert("clean", "data", vec!["t".into()], 60)
            .await
            .unwrap();

        let tool = MemoryStagedListTool { ephemeral };
        let output = tool
            .execute(serde_json::json!({"only_needs_review": true}))
            .await
            .unwrap();
        assert!(output.contains("\"staged_count\":1"));
        assert!(output.contains("\"title\":\"contested\""));
    }
}
