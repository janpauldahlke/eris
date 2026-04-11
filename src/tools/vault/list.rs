use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use tokio::fs;

use crate::executive::error::{FcpError, Result};
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct VaultListArgs {
    #[serde(alias = "path")]
    pub directory: String,
}

pub struct VaultListTool {
    pub workspace_root: PathBuf,
}

#[async_trait]
impl Tool for VaultListTool {
    fn name(&self) -> &'static str {
        "vault:list"
    }

    fn description(&self) -> &'static str {
        "Returns a flat list of file paths in a specified Vault subdirectory."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(VaultListArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: VaultListArgs = serde_json::from_value(args)
            .map_err(FcpError::ParseFault)?;

        let target_dir = self.workspace_root.join(&args.directory);

        if !target_dir.starts_with(&self.workspace_root) {
            return Err(FcpError::ToolFault { 
                tool_name: self.name().into(), 
                reason: "Path Traversal Denied".into() 
            });
        }

        let mut entries = match fs::read_dir(&target_dir).await {
            Ok(e) => e,
            Err(e) => return Err(FcpError::Io(e)),
        };

        let mut files = Vec::new();
        while let Some(entry) = entries.next_entry().await.map_err(FcpError::Io)? {
            if let Some(name) = entry.file_name().to_str() {
                // Return relative path elements for density
                files.push(name.to_string());
            }
        }

        files.sort();
        Ok(format!("SUCCESS: Directory contents:\n{}", files.join("\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use tokio::fs::{self, File};

    #[tokio::test]
    async fn test_vault_list_directory() -> Result<()> {
        let dir = tempdir().unwrap();
        let target = dir.path().join("90_Drops");
        fs::create_dir(&target).await.unwrap();
        
        File::create(target.join("a.md")).await.unwrap();
        File::create(target.join("b.md")).await.unwrap();

        let tool = VaultListTool { workspace_root: dir.path().to_path_buf() };
        let args = serde_json::json!({ "directory": "90_Drops" });

        let result = tool.execute(args).await?;
        assert!(result.contains("a.md"));
        assert!(result.contains("b.md"));
        Ok(())
    }
}
