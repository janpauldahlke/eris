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
    pub title: String,
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
        "Pulls staged content by title from ephemeral memory and persists it to the vault (disk + Qdrant). Routes to the correct vault folder based on tags (persons, user, semantic, episodic)."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(MemoryCommitArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: MemoryCommitArgs = serde_json::from_value(args)
            .map_err(FcpError::ParseFault)?;

        let entry = self.ephemeral.get_entry(&args.title).await
            .ok_or_else(|| FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: format!("No staged memory found for title: {}", args.title),
            })?;

        let target_subdir = resolve_vault_subdir(&entry.tags);

        let sanitized = args.title.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
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
            args.title, tags_yaml, now, entry.data,
        );

        tokio::fs::write(&path, frontmatter).await.map_err(FcpError::Io)?;

        self.semantic.upsert(&entry.data, entry.tags.clone()).await?;

        self.ephemeral.cache.invalidate(&args.title).await;

        tracing::info!(
            title = %args.title,
            subdir = target_subdir,
            tags = ?entry.tags,
            path = %path.display(),
            "Committed memory to vault"
        );

        Ok(format!(
            "Committed '{}' to {}/{}.md and indexed in semantic brain",
            args.title, target_subdir, sanitized
        ))
    }
}

#[cfg(test)]
mod tests {
    // Testing requires SemanticBrain, which needs a live Qdrant instance.
}
