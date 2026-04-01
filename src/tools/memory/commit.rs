use std::sync::Arc;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::executive::error::{FcpError, Result};
use crate::tools::traits::Tool;
use crate::memory::semantic::SemanticBrain;
use crate::memory::ephemeral::{EphemeralMemory, resolve_vault_subdir};

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

#[async_trait]
impl Tool for MemoryCommitTool {
    fn name(&self) -> &'static str {
        "memory:commit"
    }

    fn description(&self) -> &'static str {
        "Persists one staged memory to the vault (disk + Qdrant). Prefer staged_id; title is legacy fallback."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(MemoryCommitArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: MemoryCommitArgs = serde_json::from_value(args)
            .map_err(FcpError::ParseFault)?;

        let (entry, lookup_ref) = if let Some(staged_id) = args.staged_id.as_deref().filter(|s| !s.trim().is_empty()) {
            let entry = self.ephemeral.get_by_id(staged_id).await
                .ok_or_else(|| FcpError::ToolFault {
                    tool_name: self.name().into(),
                    reason: format!("No staged memory found for staged_id: {}", staged_id),
                })?;
            (entry, staged_id.to_string())
        } else if let Some(title) = args.title.as_deref().filter(|s| !s.trim().is_empty()) {
            let entry = self.ephemeral.get_by_title(title).await
                .ok_or_else(|| FcpError::ToolFault {
                    tool_name: self.name().into(),
                    reason: format!("No staged memory found for title: {}", title),
                })?;
            (entry, title.to_string())
        } else {
            return Err(FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "Either staged_id or title is required".to_string(),
            });
        };

        let target_subdir = resolve_vault_subdir(&entry.tags);

        let sanitized = entry.title.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        let dir = self.workspace_root.join(target_subdir);
        tokio::fs::create_dir_all(&dir).await.map_err(FcpError::Io)?;
        let path = dir.join(format!("{}.md", sanitized));

        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let tags_yaml = entry.tags.iter()
            .map(|t| format!("  - {}", t))
            .collect::<Vec<_>>()
            .join("\n");
        let frontmatter = format!(
            "---\ntitle: \"{}\"\ntags:\n{}\ncommitted_at: {}\n---\n\n{}",
            entry.title, tags_yaml, now, entry.data,
        );

        tokio::fs::write(&path, frontmatter).await.map_err(FcpError::Io)?;

        self.semantic.upsert(&entry.data, entry.tags.clone()).await?;

        self.ephemeral.cache.invalidate(&entry.staged_id).await;

        tracing::info!(
            title = %entry.title,
            staged_id = %entry.staged_id,
            lookup = %lookup_ref,
            subdir = target_subdir,
            tags = ?entry.tags,
            path = %path.display(),
            "Committed memory to vault"
        );

        Ok(format!(
            "Committed '{}' (id: {}) to {}/{}.md and indexed in semantic brain",
            entry.title, entry.staged_id, target_subdir, sanitized
        ))
    }
}

#[cfg(test)]
mod tests {
    // Testing requires SemanticBrain, which needs a live Qdrant instance.
}
