use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::executive::error::{FcpError, Result};
use crate::memory::ephemeral::{CacheValue, EphemeralMemory, is_web_artifact_staging};
use crate::memory::semantic::SemanticBrain;
use crate::memory::types::{EpistemicStatus, VaultKind};
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct MemoryCommitArgs {
    pub staged_id: Option<String>,
    pub title: Option<String>,
}

pub struct MemoryCommitTool {
    pub workspace_root: std::path::PathBuf,
    pub semantic: Arc<SemanticBrain>,
    pub ephemeral: Arc<EphemeralMemory>,
}

/// Write a revisioned vault entry using `VaultKind` routing.
/// Returns the vault-relative key (e.g. `30_Synthesis/<node_id>/r0001.md`).
pub async fn write_revisioned_vault_entry(
    workspace_root: &std::path::Path,
    entry: &CacheValue,
) -> Result<String> {
    let target_dir_name = entry.kind.dir_name();

    // 00_Invariants write deny — hard enforcement
    if target_dir_name.starts_with("00_") {
        return Err(FcpError::ToolFault {
            tool_name: "memory:commit".into(),
            reason: "Agent writes to 00_Invariants are forbidden".into(),
        });
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    match entry.kind {
        VaultKind::Synthesis => {
            // Revisioned zettel: 30_Synthesis/<node_id>/rXXXX.md
            let node_dir = workspace_root.join(target_dir_name).join(&entry.node_id);
            tokio::fs::create_dir_all(&node_dir)
                .await
                .map_err(FcpError::Io)?;

            let rev = next_revision_number(&node_dir).await;
            let filename = format!("r{:04}.md", rev);
            let path = node_dir.join(&filename);

            let frontmatter = build_frontmatter(entry, now, rev, true);
            tokio::fs::write(&path, frontmatter)
                .await
                .map_err(FcpError::Io)?;

            // Mark previous head as non-current
            if rev > 1 {
                mark_previous_revisions_non_current(&node_dir, rev).await;
            }

            let vault_key = format!("{}/{}/{}", target_dir_name, entry.node_id, filename);
            tracing::info!(
                node_id = %entry.node_id,
                rev,
                vault_key = %vault_key,
                title = %entry.title,
                "Wrote revisioned synthesis zettel"
            );
            Ok(vault_key)
        }
        VaultKind::Topology | VaultKind::Discourse => {
            // Non-revisioned: flat file in target dir
            let dir = workspace_root.join(target_dir_name);
            tokio::fs::create_dir_all(&dir)
                .await
                .map_err(FcpError::Io)?;

            let sanitized = entry
                .title
                .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
            let filename = format!("{}.md", sanitized);
            let path = dir.join(&filename);

            let frontmatter = build_frontmatter(entry, now, 1, false);
            tokio::fs::write(&path, frontmatter)
                .await
                .map_err(FcpError::Io)?;

            let vault_key = format!("{}/{}", target_dir_name, filename);
            tracing::info!(
                kind = %entry.kind,
                vault_key = %vault_key,
                title = %entry.title,
                "Wrote flat vault entry"
            );
            Ok(vault_key)
        }
    }
}

fn build_frontmatter(entry: &CacheValue, now: u64, rev: u32, is_current: bool) -> String {
    let tags_yaml = entry
        .tags
        .iter()
        .map(|t| format!("  - {}", t))
        .collect::<Vec<_>>()
        .join("\n");

    let epistemic = EpistemicStatus::default();

    let fm = format!(
        "---\ntitle: \"{}\"\nnode_id: \"{}\"\nrev: {}\nis_current: {}\nepistemic_status: \"{}\"\nkind: \"{}\"\ntags:\n{}\ncommitted_at: {}\n---\n\n{}",
        entry.title,
        entry.node_id,
        rev,
        is_current,
        epistemic,
        entry.kind,
        tags_yaml,
        now,
        entry.data,
    );
    fm
}

/// Scan a node directory and return the next revision number.
async fn next_revision_number(node_dir: &std::path::Path) -> u32 {
    let mut max_rev = 0u32;
    if let Ok(mut entries) = tokio::fs::read_dir(node_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(num_str) = name.strip_prefix('r').and_then(|s| s.strip_suffix(".md"))
                && let Ok(n) = num_str.parse::<u32>()
            {
                max_rev = max_rev.max(n);
            }
        }
    }
    max_rev + 1
}

/// Set `is_current: false` in frontmatter of older revisions (best-effort text replacement).
async fn mark_previous_revisions_non_current(node_dir: &std::path::Path, current_rev: u32) {
    if let Ok(mut entries) = tokio::fs::read_dir(node_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if let Some(num_str) = name_str
                .strip_prefix('r')
                .and_then(|s| s.strip_suffix(".md"))
                && let Ok(n) = num_str.parse::<u32>()
                && n < current_rev
            {
                let path = entry.path();
                if let Ok(content) = tokio::fs::read_to_string(&path).await {
                    let updated = content.replace("is_current: true", "is_current: false");
                    if updated != content {
                        let _ = tokio::fs::write(&path, updated).await;
                    }
                }
            }
        }
    }
}

#[async_trait]
impl Tool for MemoryCommitTool {
    fn name(&self) -> &'static str {
        "memory:commit"
    }

    fn description(&self) -> &'static str {
        "Persists one staged memory to the vault (disk + Qdrant). Uses kind-based routing and revisioned zettels for synthesis nodes."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(MemoryCommitArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: MemoryCommitArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;

        let (entry, lookup_ref) =
            if let Some(staged_id) = args.staged_id.as_deref().filter(|s| !s.trim().is_empty()) {
                let entry = self.ephemeral.get_by_id(staged_id).await.ok_or_else(|| {
                    FcpError::ToolFault {
                        tool_name: self.name().into(),
                        reason: format!("No staged memory found for staged_id: {}", staged_id),
                    }
                })?;
                (entry, staged_id.to_string())
            } else if let Some(title) = args.title.as_deref().filter(|s| !s.trim().is_empty()) {
                let entry = self.ephemeral.get_by_title(title).await.ok_or_else(|| {
                    FcpError::ToolFault {
                        tool_name: self.name().into(),
                        reason: format!("No staged memory found for title: {}", title),
                    }
                })?;
                (entry, title.to_string())
            } else {
                return Err(FcpError::ToolFault {
                    tool_name: self.name().into(),
                    reason: "Either staged_id or title is required".to_string(),
                });
            };

        if is_web_artifact_staging(&entry.tags, &entry.title) {
            self.ephemeral.cache.invalidate(&entry.staged_id).await;
            tracing::info!(
                staged_id = %entry.staged_id,
                title = %entry.title,
                "Web artifact staging cleared; vault write skipped (content already chunked in semantic index)"
            );
            return Ok(
                "SUCCESS: Web artifact kept in semantic index only; vault write skipped."
                    .to_string(),
            );
        }

        let vault_key = write_revisioned_vault_entry(&self.workspace_root, &entry).await?;

        self.semantic
            .upsert(&entry.data, entry.tags.clone(), Some(vault_key.clone()))
            .await?;

        self.ephemeral.cache.invalidate(&entry.staged_id).await;

        tracing::info!(
            title = %entry.title,
            staged_id = %entry.staged_id,
            lookup = %lookup_ref,
            node_id = %entry.node_id,
            kind = %entry.kind,
            vault_key = %vault_key,
            tags = ?entry.tags,
            "Committed memory to vault"
        );

        Ok(format!(
            "Committed '{}' (node_id={}, kind={}) to {} and indexed in semantic brain",
            entry.title, entry.node_id, entry.kind, vault_key,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_write_revisioned_creates_synthesis_zettel() {
        let temp_dir = tempfile::tempdir().unwrap();
        let workspace_root = temp_dir.path();

        let entry = CacheValue {
            staged_id: "s1".into(),
            title: "test_concept".into(),
            data: "Atomic zettel content".into(),
            tags: vec!["test".into()],
            expires_at: u64::MAX,
            node_id: "abc-123".into(),
            canonical_key: "test_concept".into(),
            tier: crate::memory::types::EphemeralTier::Promote,
            promotion_score: 10.0,
            mention_count: 3,
            needs_review: false,
            first_seen_at: 1000,
            last_seen_at: 2000,
            kind: VaultKind::Synthesis,
        };

        let vault_key = write_revisioned_vault_entry(workspace_root, &entry)
            .await
            .unwrap();
        assert!(vault_key.starts_with("30_Synthesis/abc-123/r0001.md"));

        // Check file exists
        let path = workspace_root.join("30_Synthesis/abc-123/r0001.md");
        assert!(path.exists());
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.contains("is_current: true"));
        assert!(content.contains("node_id: \"abc-123\""));
        assert!(content.contains("rev: 1"));
    }

    #[tokio::test]
    async fn test_write_revisioned_increments_revision() {
        let temp_dir = tempfile::tempdir().unwrap();
        let workspace_root = temp_dir.path();

        let entry = CacheValue {
            staged_id: "s1".into(),
            title: "evolving".into(),
            data: "version 1".into(),
            tags: vec!["test".into()],
            expires_at: u64::MAX,
            node_id: "node-evolve".into(),
            canonical_key: "evolving".into(),
            tier: crate::memory::types::EphemeralTier::Promote,
            promotion_score: 10.0,
            mention_count: 1,
            needs_review: false,
            first_seen_at: 1000,
            last_seen_at: 1000,
            kind: VaultKind::Synthesis,
        };

        write_revisioned_vault_entry(workspace_root, &entry)
            .await
            .unwrap();

        let entry_v2 = CacheValue {
            data: "version 2".into(),
            ..entry
        };
        let key2 = write_revisioned_vault_entry(workspace_root, &entry_v2)
            .await
            .unwrap();
        assert!(key2.contains("r0002.md"));

        // r0001 should now have is_current: false
        let r1_content =
            tokio::fs::read_to_string(workspace_root.join("30_Synthesis/node-evolve/r0001.md"))
                .await
                .unwrap();
        assert!(r1_content.contains("is_current: false"));
    }

    #[test]
    fn test_write_denies_invariants() {
        // VaultKind does not have an Invariants variant, so this is enforced by design.
        // Verify dir_name never maps to 00_.
        assert!(!VaultKind::Topology.dir_name().starts_with("00_"));
        assert!(!VaultKind::Discourse.dir_name().starts_with("00_"));
        assert!(!VaultKind::Synthesis.dir_name().starts_with("00_"));
    }

    #[tokio::test]
    async fn test_write_topology_flat_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let workspace_root = temp_dir.path();

        let entry = CacheValue {
            staged_id: "s1".into(),
            title: "discord_config".into(),
            data: "gateway intents here".into(),
            tags: vec!["infra".into()],
            expires_at: u64::MAX,
            node_id: "node-topo".into(),
            canonical_key: "discord_config".into(),
            tier: crate::memory::types::EphemeralTier::Promote,
            promotion_score: 10.0,
            mention_count: 1,
            needs_review: false,
            first_seen_at: 1000,
            last_seen_at: 1000,
            kind: VaultKind::Topology,
        };

        let vault_key = write_revisioned_vault_entry(workspace_root, &entry)
            .await
            .unwrap();
        assert_eq!(vault_key, "10_Topology/discord_config.md");
        assert!(
            workspace_root
                .join("10_Topology/discord_config.md")
                .exists()
        );
    }
}
