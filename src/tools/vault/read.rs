use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use tokio::fs;

use crate::executive::error::{FcpError, Result};
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct VaultReadArgs {
    /// Relative path inside the workspace vault (e.g. '10_Projects/fcp.md')
    pub relative_path: String,
}

pub struct VaultReadTool {
    pub workspace_root: PathBuf,
}

#[async_trait]
impl Tool for VaultReadTool {
    fn name(&self) -> &'static str {
        "vault:read"
    }

    fn description(&self) -> &'static str {
        "Reads a file from the vault. If the file is too large (>3000 tokens), returns a semantic map of headers instead."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(VaultReadArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: VaultReadArgs = serde_json::from_value(args)
            .map_err(|e| FcpError::ParseFault(e))?;

        // Gatekeeper path firewall is optional for read, but we should prevent absolute traversal
        let target_path = self.workspace_root.join(&args.relative_path);
        if !target_path.starts_with(&self.workspace_root) {
             return Err(FcpError::ToolFault { 
                 tool_name: self.name().into(), 
                 reason: "Path Traversal Denied".into() 
             });
        }

        let content = fs::read_to_string(&target_path).await
            .map_err(|e| FcpError::Io(e))?;

        // Token estimation (~4 chars per token constraint limit)
        let estimated_tokens = content.chars().count() / 4;
        
        if estimated_tokens > 3000 {
            let headers: Vec<&str> = content.lines()
                .filter(|line| line.trim_start().starts_with('#'))
                .collect();
                
            let map = headers.join("\n");
            return Ok(format!(
                "ERROR: File exceeds 3000 tokens. Use memory:query to search it semantically. FILE MAP:\n{}",
                map
            ));
        }

        Ok(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use tokio::fs;

    #[tokio::test]
    async fn test_vault_read_normal() -> Result<()> {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("normal.md");
        fs::write(&file_path, "Hello, Vault!").await.unwrap();

        let tool = VaultReadTool { workspace_root: dir.path().to_path_buf() };
        let args = serde_json::json!({ "relative_path": "normal.md" });

        let result = tool.execute(args).await?;
        assert_eq!(result, "Hello, Vault!");
        Ok(())
    }

    #[tokio::test]
    async fn test_vault_read_exceeds_tokens() -> Result<()> {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("massive.md");
        
        let mut massive_content = String::from("# Header 1\n");
        massive_content.push_str(&"A".repeat(12001)); // > 3000 tokens (12000 chars)
        massive_content.push_str("\n## Header 2\n");
        massive_content.push_str(&"B".repeat(10));
        
        fs::write(&file_path, massive_content).await.unwrap();

        let tool = VaultReadTool { workspace_root: dir.path().to_path_buf() };
        let args = serde_json::json!({ "relative_path": "massive.md" });

        let result = tool.execute(args).await?;
        assert!(result.starts_with("ERROR: File exceeds 3000 tokens."));
        assert!(result.contains("# Header 1"));
        assert!(result.contains("## Header 2"));
        Ok(())
    }
}
