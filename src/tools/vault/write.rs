use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;

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
#[schemars(description = "The system will automatically route raw filenames to the correct taxonomy folder (e.g., 10_Episodic).")]
pub struct VaultWriteArgs {
    pub relative_path: String,
    pub content: String,
    pub mode: WriteMode,
}

pub struct VaultWriteTool {
    pub workspace_root: PathBuf,
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
        let args: VaultWriteArgs = serde_json::from_value(args)
            .map_err(|e| FcpError::ParseFault(e))?;

        let mut path = PathBuf::from(&args.relative_path);

        if path.parent().map_or(true, |p| p.as_os_str().is_empty()) {
            let filename = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            let extension = path.extension().unwrap_or_default().to_string_lossy().to_lowercase();

            let target_dir = if ["png", "jpg", "jpeg", "gif", "pdf", "csv", "json"].contains(&extension.as_str()) {
                "30_Assets"
            } else if filename.starts_with("sys_") || filename.starts_with("core_") || filename.starts_with("identity") {
                "00_Core"
            } else if filename.starts_with("user_") || filename.starts_with("pref_") {
                "40_User"
            } else {
                "10_Episodic"
            };
            path = PathBuf::from(target_dir).join(path);
        }

        let final_relative_path_string = path.to_string_lossy().to_string();

        validate_path_is_mutable(&final_relative_path_string)?;

        let target_path = self.workspace_root.join(&path);
        
        // Ensure parent directories exist
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| FcpError::Io(e))?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(args.mode == WriteMode::Overwrite)
            .append(args.mode == WriteMode::Append)
            .open(&target_path)
            .await
            .map_err(|e| FcpError::Io(e))?;

        file.write_all(args.content.as_bytes()).await.map_err(|e| FcpError::Io(e))?;
        file.flush().await.map_err(|e| FcpError::Io(e))?;

        Ok(format!("SUCCESS: File written and routed to {}", final_relative_path_string))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_vault_write_overwrite() -> Result<()> {
        let dir = tempdir().unwrap();
        let tool = VaultWriteTool { workspace_root: dir.path().to_path_buf() };
        
        let args = serde_json::json!({
            "relative_path": "test.md",
            "content": "Initial",
            "mode": "overwrite"
        });
        
        let result = tool.execute(args.clone()).await?;
        assert_eq!(result, "SUCCESS: File written and routed to 10_Episodic/test.md");
        
        let written = fs::read_to_string(dir.path().join("10_Episodic/test.md")).await.unwrap();
        assert_eq!(written, "Initial");
        Ok(())
    }

    #[tokio::test]
    async fn test_vault_write_gatekeeper_block() -> Result<()> {
        let dir = tempdir().unwrap();
        let tool = VaultWriteTool { workspace_root: dir.path().to_path_buf() };
        
        let args = serde_json::json!({
            "relative_path": "core_Identity.md",
            "content": "Malicious",
            "mode": "overwrite"
        });
        
        let result = tool.execute(args).await;
        assert!(result.is_err());
        Ok(())
    }
}
