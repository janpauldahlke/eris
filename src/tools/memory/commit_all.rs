use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::executive::error::{FcpError, Result};
use crate::memory::ephemeral::{is_web_artifact_staging, resolve_vault_subdir, EphemeralMemory};
use crate::memory::semantic::SemanticBrain;
use crate::tools::traits::Tool;
use crate::config::MemoryRoutingConfig;

#[derive(Deserialize, JsonSchema, Default)]
pub struct MemoryCommitAllArgs {}

#[derive(Serialize)]
struct CommitAllResponse {
    committed: Vec<String>,
    skipped: Vec<String>,
    indexing_failed: Vec<String>,
    committed_count: usize,
    skipped_count: usize,
    indexing_failed_count: usize,
}

pub struct MemoryCommitAllTool {
    pub workspace_root: std::path::PathBuf,
    pub semantic: Arc<SemanticBrain>,
    pub ephemeral: Arc<EphemeralMemory>,
    pub memory_routing: MemoryRoutingConfig,
}

#[async_trait]
impl Tool for MemoryCommitAllTool {
    fn name(&self) -> &'static str {
        "memory:commit_all"
    }

    fn description(&self) -> &'static str {
        "Commits all currently staged memories with best effort. Invalid entries are skipped."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(MemoryCommitAllArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let _args: MemoryCommitAllArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let entries = self.ephemeral.list_entries();

        let mut committed = Vec::new();
        let mut skipped = Vec::new();
        let mut indexing_failed = Vec::new();

        for entry in entries {
            if entry.title.trim().is_empty() || entry.data.trim().is_empty() || entry.tags.is_empty() {
                skipped.push(entry.staged_id.clone());
                continue;
            }

            if is_web_artifact_staging(&entry.tags, &entry.title) {
                self.ephemeral.cache.invalidate(&entry.staged_id).await;
                committed.push(entry.staged_id.clone());
                tracing::info!(
                    staged_id = %entry.staged_id,
                    title = %entry.title,
                    "Web artifact: vault write skipped, ephemeral cleared (semantic chunks from fetch)"
                );
                continue;
            }

            let target_subdir = resolve_vault_subdir(&entry.tags, &self.memory_routing);
            let sanitized = entry
                .title
                .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
            let dir = self.workspace_root.join(target_subdir);
            if let Err(e) = tokio::fs::create_dir_all(&dir).await {
                tracing::warn!(staged_id = %entry.staged_id, error = %e, "Skipping staged entry; failed to create target dir");
                skipped.push(entry.staged_id.clone());
                continue;
            }

            let path = dir.join(format!("{}.md", sanitized));
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
            let tags_yaml = entry
                .tags
                .iter()
                .map(|t| format!("  - {}", t))
                .collect::<Vec<_>>()
                .join("\n");
            let frontmatter = format!(
                "---\ntitle: \"{}\"\ntags:\n{}\ncommitted_at: {}\n---\n\n{}",
                entry.title, tags_yaml, now, entry.data,
            );

            if let Err(e) = tokio::fs::write(&path, frontmatter).await {
                tracing::warn!(staged_id = %entry.staged_id, error = %e, "Skipping staged entry; failed to write vault file");
                skipped.push(entry.staged_id.clone());
                continue;
            }

            let vault_key = path
                .strip_prefix(&self.workspace_root)
                .ok()
                .and_then(|p| p.to_str())
                .map(|s| s.replace('\\', "/"));

            if let Err(e) = self
                .semantic
                .upsert(&entry.data, entry.tags.clone(), vault_key)
                .await
            {
                tracing::warn!(staged_id = %entry.staged_id, error = %e, "Vault write succeeded but semantic indexing failed");
                indexing_failed.push(entry.staged_id.clone());
                continue;
            }

            self.ephemeral.cache.invalidate(&entry.staged_id).await;
            committed.push(entry.staged_id.clone());
        }

        let response = CommitAllResponse {
            committed_count: committed.len(),
            skipped_count: skipped.len(),
            indexing_failed_count: indexing_failed.len(),
            committed,
            skipped,
            indexing_failed,
        };

        serde_json::to_string(&response).map_err(|e| FcpError::Config(e.to_string()))
    }
}
