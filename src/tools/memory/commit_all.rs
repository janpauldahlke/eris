use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

use crate::executive::error::{FcpError, Result};
use crate::memory::ephemeral::{EphemeralMemory, is_web_artifact_staging};
use crate::memory::semantic::SemanticBrain;
use crate::memory::types::EphemeralTier;
use crate::tools::traits::Tool;

use super::commit::write_revisioned_vault_entry;

#[derive(Deserialize, JsonSchema, Default)]
pub struct MemoryCommitAllArgs {}

#[derive(Serialize)]
struct CommitAllResponse {
    committed: Vec<String>,
    skipped_not_promote: Vec<String>,
    skipped_invalid: Vec<String>,
    indexing_failed: Vec<String>,
    committed_count: usize,
    skipped_not_promote_count: usize,
    skipped_invalid_count: usize,
    indexing_failed_count: usize,
}

pub struct MemoryCommitAllTool {
    pub workspace_root: std::path::PathBuf,
    pub semantic: Arc<SemanticBrain>,
    pub ephemeral: Arc<EphemeralMemory>,
}

#[async_trait]
impl Tool for MemoryCommitAllTool {
    fn name(&self) -> &'static str {
        "memory:commit_all"
    }

    fn description(&self) -> &'static str {
        "Commits all promote-tier staged memories. Session and scratch entries are skipped."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(MemoryCommitAllArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let _args: MemoryCommitAllArgs =
            serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let entries = self.ephemeral.list_entries();

        let mut committed = Vec::new();
        let mut skipped_not_promote = Vec::new();
        let mut skipped_invalid = Vec::new();
        let mut indexing_failed = Vec::new();

        for entry in entries {
            // Promote-only gate
            if entry.tier != EphemeralTier::Promote {
                tracing::debug!(
                    staged_id = %entry.staged_id,
                    title = %entry.title,
                    tier = %entry.tier,
                    "commit_all: skipping non-promote entry"
                );
                skipped_not_promote.push(format!("{}(tier={})", entry.staged_id, entry.tier));
                continue;
            }

            if entry.title.trim().is_empty()
                || entry.data.trim().is_empty()
                || entry.tags.is_empty()
            {
                skipped_invalid.push(entry.staged_id.clone());
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

            // Revisioned vault write
            match write_revisioned_vault_entry(&self.workspace_root, &entry).await {
                Ok(vault_key) => {
                    if let Err(e) = self
                        .semantic
                        .upsert(&entry.data, entry.tags.clone(), Some(vault_key))
                        .await
                    {
                        tracing::warn!(
                            staged_id = %entry.staged_id,
                            error = %e,
                            "Vault write succeeded but semantic indexing failed"
                        );
                        indexing_failed.push(entry.staged_id.clone());
                        continue;
                    }

                    self.ephemeral.cache.invalidate(&entry.staged_id).await;
                    committed.push(entry.staged_id.clone());
                }
                Err(e) => {
                    tracing::warn!(
                        staged_id = %entry.staged_id,
                        error = %e,
                        "Skipping staged entry; vault write failed"
                    );
                    skipped_invalid.push(entry.staged_id.clone());
                }
            }
        }

        tracing::info!(
            committed = committed.len(),
            skipped_not_promote = skipped_not_promote.len(),
            skipped_invalid = skipped_invalid.len(),
            indexing_failed = indexing_failed.len(),
            "commit_all complete (promote-only)"
        );

        let response = CommitAllResponse {
            committed_count: committed.len(),
            skipped_not_promote_count: skipped_not_promote.len(),
            skipped_invalid_count: skipped_invalid.len(),
            indexing_failed_count: indexing_failed.len(),
            committed,
            skipped_not_promote,
            skipped_invalid,
            indexing_failed,
        };

        serde_json::to_string(&response).map_err(|e| FcpError::Config(e.to_string()))
    }
}
