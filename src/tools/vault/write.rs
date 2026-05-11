use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;

use super::taglist_cache::TaglistCache;
use super::taglist_index::is_synthesis_md_path;
use crate::executive::error::{FcpError, Result};
use crate::tools::traits::Tool;
use crate::tools::validation::validate_path_is_mutable;

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum WriteMode {
    Overwrite,
    Append,
}

#[derive(Deserialize, JsonSchema)]
#[schemars(
    description = "The system will automatically route raw filenames to the correct taxonomy folder (e.g., 10_Episodic)."
)]
pub struct VaultWriteArgs {
    pub relative_path: String,
    pub content: String,
    pub mode: WriteMode,
}

pub struct VaultWriteTool {
    pub workspace_root: PathBuf,
    pub max_content_chars: usize,
    /// Shared with `VaultTaglistTool`: flipped on successful writes under `30_Synthesis/*.md` so
    /// the next `vault:taglist` call rebuilds and persists.
    pub taglist_cache: Arc<TaglistCache>,
}

#[async_trait]
impl Tool for VaultWriteTool {
    fn name(&self) -> &'static str {
        "vault:write"
    }

    fn description(&self) -> &'static str {
        "Writes strings directly to the physical disk inside the workspace. The system will automatically route raw filenames to the correct taxonomy folder (e.g., 10_Episodic)."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(VaultWriteArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: VaultWriteArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;

        if args.content.len() > self.max_content_chars {
            return Err(FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: format!(
                    "Content exceeds max size ({} chars > {} limit). Split into smaller files.",
                    args.content.len(),
                    self.max_content_chars,
                ),
            });
        }

        let mut path = PathBuf::from(&args.relative_path);

        if path.parent().is_none_or(|p| p.as_os_str().is_empty()) {
            let mut target_dir = None;

            if args.content.starts_with("---")
                && let Some(end_idx) = args.content[3..].find("---")
            {
                let frontmatter = &args.content[..3 + end_idx + 3];
                if frontmatter.contains("00_Core") {
                    target_dir = Some("00_Core");
                } else if frontmatter.contains("10_Episodic") {
                    target_dir = Some("10_Episodic");
                } else if frontmatter.contains("30_Assets") {
                    target_dir = Some("30_Assets");
                } else if frontmatter.contains("40_User") {
                    target_dir = Some("40_User");
                }
            }

            let target_dir = target_dir.unwrap_or_else(|| {
                let filename = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let extension = path
                    .extension()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase();

                if ["png", "jpg", "jpeg", "gif", "pdf", "csv", "json"].contains(&extension.as_str())
                {
                    "30_Assets"
                } else if filename.starts_with("user_") || filename.starts_with("pref_") {
                    "40_User"
                } else {
                    "10_Episodic"
                }
            });

            path = PathBuf::from(target_dir).join(path);
        }

        let final_relative_path_string = path.to_string_lossy().to_string();

        validate_path_is_mutable(&final_relative_path_string)?;

        let target_path = self.workspace_root.join(&path);

        // Ensure parent directories exist
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent).await.map_err(FcpError::Io)?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(args.mode == WriteMode::Overwrite)
            .append(args.mode == WriteMode::Append)
            .open(&target_path)
            .await
            .map_err(FcpError::Io)?;

        file.write_all(args.content.as_bytes())
            .await
            .map_err(FcpError::Io)?;
        file.flush().await.map_err(FcpError::Io)?;

        if is_synthesis_md_path(&final_relative_path_string) {
            self.taglist_cache.mark_dirty();
        }

        Ok(format!(
            "SUCCESS: File written and routed to {}",
            final_relative_path_string
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_vault_write_overwrite() -> Result<()> {
        let dir = tempdir().unwrap();
        let tool = VaultWriteTool {
            workspace_root: dir.path().to_path_buf(),
            max_content_chars: 100_000,
            taglist_cache: TaglistCache::into_arc(),
        };

        let args = serde_json::json!({
            "relative_path": "test.md",
            "content": "Initial",
            "mode": "overwrite"
        });

        let result = tool.execute(args.clone()).await?;
        assert_eq!(
            result,
            "SUCCESS: File written and routed to 10_Episodic/test.md"
        );

        let written = fs::read_to_string(dir.path().join("10_Episodic/test.md"))
            .await
            .unwrap();
        assert_eq!(written, "Initial");
        Ok(())
    }

    #[tokio::test]
    async fn test_vault_write_gatekeeper_block() -> Result<()> {
        let dir = tempdir().unwrap();
        let tool = VaultWriteTool {
            workspace_root: dir.path().to_path_buf(),
            max_content_chars: 100_000,
            taglist_cache: TaglistCache::into_arc(),
        };

        let args = serde_json::json!({
            "relative_path": "00_Core/Identity.md",
            "content": "Malicious",
            "mode": "overwrite"
        });

        let result = tool.execute(args).await;
        assert!(result.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn test_vault_write_yaml_frontmatter_override() -> Result<()> {
        let dir = tempdir().unwrap();
        let tool = VaultWriteTool {
            workspace_root: dir.path().to_path_buf(),
            max_content_chars: 100_000,
            taglist_cache: TaglistCache::into_arc(),
        };

        let args = serde_json::json!({
            "relative_path": "test_image.png",
            "content": "---\ntags:\n  - 10_Episodic/visuals\n---\n...",
            "mode": "overwrite"
        });

        let result = tool.execute(args.clone()).await?;
        assert_eq!(
            result,
            "SUCCESS: File written and routed to 10_Episodic/test_image.png"
        );

        let written = fs::read_to_string(dir.path().join("10_Episodic/test_image.png"))
            .await
            .unwrap();
        assert_eq!(written, "---\ntags:\n  - 10_Episodic/visuals\n---\n...");
        Ok(())
    }

    #[tokio::test]
    async fn synthesis_write_marks_taglist_cache_dirty() -> Result<()> {
        let dir = tempdir().unwrap();
        let cache = TaglistCache::into_arc();
        let tool = VaultWriteTool {
            workspace_root: dir.path().to_path_buf(),
            max_content_chars: 100_000,
            taglist_cache: Arc::clone(&cache),
        };

        let args = serde_json::json!({
            "relative_path": "30_Synthesis/uuid-1/r0001.md",
            "content": "---\ntags:\n  - sandbox\n---\nhello",
            "mode": "overwrite"
        });

        let _ = tool.execute(args).await?;
        assert!(cache.is_dirty());
        Ok(())
    }

    #[tokio::test]
    async fn non_synthesis_write_does_not_dirty_taglist_cache() -> Result<()> {
        let dir = tempdir().unwrap();
        let cache = TaglistCache::into_arc();
        let tool = VaultWriteTool {
            workspace_root: dir.path().to_path_buf(),
            max_content_chars: 100_000,
            taglist_cache: Arc::clone(&cache),
        };

        let args = serde_json::json!({
            "relative_path": "20_Discourse/Tasks.md",
            "content": "## row\nresult: ok",
            "mode": "append"
        });

        let _ = tool.execute(args).await?;
        assert!(!cache.is_dirty(), "expected non-synthesis write to leave taglist clean");
        Ok(())
    }
}
