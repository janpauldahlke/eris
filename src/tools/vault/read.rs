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
    pub read_limit: usize,
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
        let args: VaultReadArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;

        // Gatekeeper path firewall is optional for read, but we should prevent absolute traversal
        let target_path = self.workspace_root.join(&args.relative_path);
        if !target_path.starts_with(&self.workspace_root) {
            return Err(FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "Path Traversal Denied".into(),
            });
        }

        let mut content = fs::read_to_string(&target_path)
            .await
            .map_err(FcpError::Io)?;

        let max_bytes = self.read_limit * 4;

        if content.len() > max_bytes {
            let headers: Vec<&str> = content
                .lines()
                .filter(|line| line.trim_start().starts_with('#'))
                .collect();

            let map = headers.join("\n");

            let mut limit = max_bytes;
            while limit > 0 && !content.is_char_boundary(limit) {
                limit -= 1;
            }
            content.truncate(limit);

            content.push_str(&format!(
                "\n\n[SYSTEM WARNING: CONTENT TRUNCATED TO {} TOKENS. Use memory:query to search it semantically. FILE MAP:\n{}]",
                self.read_limit, map
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

    #[tokio::test(flavor = "current_thread")]
    async fn test_vault_read_normal() -> Result<()> {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("normal.md");
        fs::write(&file_path, "Hello, Vault!").await.unwrap();

        let tool = VaultReadTool {
            workspace_root: dir.path().to_path_buf(),
            read_limit: 3000,
        };
        let args = serde_json::json!({ "relative_path": "normal.md" });

        let result = tool.execute(args).await?;
        assert_eq!(result, "Hello, Vault!");
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_vault_read_exceeds_tokens() -> Result<()> {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("massive.md");

        let mut massive_content = String::from("# Header 1\n");
        massive_content.push_str(&"A".repeat(12001)); // > 3000 tokens (12000 chars)
        massive_content.push_str("\n## Header 2\n");
        massive_content.push_str(&"B".repeat(10));

        fs::write(&file_path, massive_content).await.unwrap();

        let tool = VaultReadTool {
            workspace_root: dir.path().to_path_buf(),
            read_limit: 3000,
        };
        let args = serde_json::json!({ "relative_path": "massive.md" });

        let result = tool.execute(args).await?;
        assert!(result.contains("[SYSTEM WARNING: CONTENT TRUNCATED TO 3000 TOKENS."));
        assert!(result.contains("# Header 1"));
        assert!(result.contains("## Header 2"));
        Ok(())
    }
}
