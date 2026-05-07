use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use crate::config::AppConfig;
use crate::executive::error::{FcpError, Result};
use crate::memory::ephemeral::{EphemeralMemory, normalize_canonical_key};
use crate::memory::types::{EphemeralTier, VaultKind};
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct MemoryStageArgs {
    /// A short descriptive title (e.g. "hagbard_profile", "weather_api"). Required.
    pub title: Option<String>,
    /// The text content to remember. Required.
    pub content: Option<String>,
    /// Free-form tags for classification. Required, at least one tag.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Optional tier override for fast-lane promotion. One of: "session", "scratch", "promote".
    /// When omitted, defaults to "session" (or preserves existing tier on upsert).
    #[serde(default)]
    pub tier: Option<EphemeralTier>,
    /// Optional vault kind hint. One of: "topology", "discourse", "synthesis".
    /// When omitted, defaults to "synthesis".
    #[serde(default)]
    pub kind: Option<VaultKind>,
}

pub struct MemoryStageTool {
    pub ephemeral: Arc<EphemeralMemory>,
    pub config: Arc<AppConfig>,
    pub max_content_chars: usize,
}

#[async_trait]
impl Tool for MemoryStageTool {
    fn name(&self) -> &'static str {
        "memory:stage"
    }

    fn description(&self) -> &'static str {
        "Stages content into ephemeral memory with title, tags, and TTL. Supports tier fast-lane via optional 'tier' field. Deduplicates by canonical_key; upserts bump score."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(MemoryStageArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: MemoryStageArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;

        let title = args
            .title
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "title is required — provide a short descriptive title (e.g. \"hagbard_profile\", \"weather_api\")".into(),
            })?;

        let content = args
            .content
            .filter(|c| !c.trim().is_empty())
            .ok_or_else(|| FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "content is required — provide the actual text to remember".into(),
            })?;

        let tags = args
            .tags
            .filter(|t| !t.is_empty())
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

        let requested_tier = args.tier.unwrap_or(EphemeralTier::Session);
        let requested_kind = args.kind.unwrap_or(VaultKind::Synthesis);
        let canonical = normalize_canonical_key(&title);

        // Check for existing entry with same canonical_key (upsert path)
        if let Some(existing) = self.ephemeral.get_by_canonical_key(&canonical).await {
            let effective_tier = std::cmp::max(existing.tier, requested_tier);
            let ttl = self.config.ttl_for_tier(effective_tier);
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            // Invalidate old entry, insert updated one
            self.ephemeral.cache.invalidate(&existing.staged_id).await;
            let new_score = existing.promotion_score + self.config.promotion_stage_boost;
            let new_count = existing.mention_count.saturating_add(1);
            let new_id = uuid::Uuid::new_v4().to_string();

            let updated = crate::memory::ephemeral::CacheValue {
                staged_id: new_id.clone(),
                title: title.clone(),
                data: content.clone(),
                tags: tags.clone(),
                expires_at: now_secs + ttl,
                node_id: existing.node_id.clone(),
                canonical_key: canonical.clone(),
                tier: effective_tier,
                promotion_score: new_score,
                mention_count: new_count,
                needs_review: existing.needs_review,
                first_seen_at: existing.first_seen_at,
                last_seen_at: now_secs,
                kind: requested_kind,
            };
            self.ephemeral.cache.insert(new_id, updated).await;

            tracing::info!(
                title = %title,
                node_id = %existing.node_id,
                canonical_key = %canonical,
                tier = %effective_tier,
                score = new_score,
                mentions = new_count,
                "memory:stage upsert (existing canonical_key)"
            );

            return Ok(format!(
                "Upserted '{}' (node_id={}, tier={}, score={:.1}, mentions={}, ttl={}s)",
                title, existing.node_id, effective_tier, new_score, new_count, ttl,
            ));
        }

        // Fresh insert
        let ttl = self.config.ttl_for_tier(requested_tier);
        let staged = self
            .ephemeral
            .insert_with_tier(
                &title,
                &content,
                tags.clone(),
                ttl,
                requested_tier,
                requested_kind,
            )
            .await?;

        // Apply stage boost to fresh entries
        if self.config.promotion_stage_boost > 0.0 {
            let boosted = crate::memory::ephemeral::CacheValue {
                promotion_score: staged.promotion_score + self.config.promotion_stage_boost,
                ..staged.clone()
            };
            self.ephemeral
                .cache
                .insert(staged.staged_id.clone(), boosted)
                .await;
        }

        tracing::info!(
            title = %title,
            staged_id = %staged.staged_id,
            node_id = %staged.node_id,
            tier = %requested_tier,
            kind = %requested_kind,
            ttl_secs = ttl,
            "memory:stage fresh insert"
        );

        Ok(format!(
            "Staged '{}' as id '{}' (tier={}, kind={}, tags={:?}, ttl={}s)",
            title, staged.staged_id, requested_tier, requested_kind, tags, ttl,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Arc<AppConfig> {
        Arc::new(AppConfig::default())
    }

    #[tokio::test]
    async fn test_memory_stage_execution() {
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let tool = MemoryStageTool {
            ephemeral: ephemeral.clone(),
            config: test_config(),
            max_content_chars: 10_000,
        };
        let args = serde_json::json!({
            "title": "infrastructure",
            "content": "The database uses port 5432.",
            "tags": ["db", "postgres"]
        });

        let result = tool.execute(args).await;
        assert!(result.is_ok());
        let entry = ephemeral.get_by_title("infrastructure").await.unwrap();
        assert_eq!(entry.data, "The database uses port 5432.");
        assert_eq!(entry.tags, vec!["db", "postgres"]);
        assert_eq!(entry.tier, EphemeralTier::Session);
    }

    #[tokio::test]
    async fn test_memory_stage_upsert_bumps_score() {
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let config = test_config();
        let tool = MemoryStageTool {
            ephemeral: ephemeral.clone(),
            config: config.clone(),
            max_content_chars: 10_000,
        };

        // First stage
        tool.execute(serde_json::json!({
            "title": "repeat_topic",
            "content": "version 1",
            "tags": ["test"]
        }))
        .await
        .unwrap();

        // Second stage (upsert)
        tool.execute(serde_json::json!({
            "title": "repeat_topic",
            "content": "version 2",
            "tags": ["test"]
        }))
        .await
        .unwrap();

        let entry = ephemeral
            .get_by_canonical_key("repeat_topic")
            .await
            .unwrap();
        assert_eq!(entry.data, "version 2");
        assert_eq!(entry.mention_count, 2);
        assert!(entry.promotion_score >= config.promotion_stage_boost * 2.0 - 0.01);
    }

    #[tokio::test]
    async fn test_memory_stage_tier_fastlane() {
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let tool = MemoryStageTool {
            ephemeral: ephemeral.clone(),
            config: test_config(),
            max_content_chars: 10_000,
        };
        let args = serde_json::json!({
            "title": "important_fact",
            "content": "Skip the ladder",
            "tags": ["critical"],
            "tier": "promote"
        });

        tool.execute(args).await.unwrap();
        let entry = ephemeral
            .get_by_canonical_key("important_fact")
            .await
            .unwrap();
        assert_eq!(entry.tier, EphemeralTier::Promote);
    }

    #[tokio::test]
    async fn test_memory_stage_kind_routing() {
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let tool = MemoryStageTool {
            ephemeral: ephemeral.clone(),
            config: test_config(),
            max_content_chars: 10_000,
        };
        let args = serde_json::json!({
            "title": "discord_config",
            "content": "gateway intents",
            "tags": ["infra"],
            "kind": "topology"
        });

        tool.execute(args).await.unwrap();
        let entry = ephemeral
            .get_by_canonical_key("discord_config")
            .await
            .unwrap();
        assert_eq!(entry.kind, VaultKind::Topology);
    }

    #[tokio::test]
    async fn test_memory_stage_rejects_null_title() {
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let tool = MemoryStageTool {
            ephemeral: ephemeral.clone(),
            config: test_config(),
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
            config: test_config(),
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
            config: test_config(),
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
            config: test_config(),
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
