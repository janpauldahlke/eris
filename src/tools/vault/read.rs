use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use tokio::fs;

use crate::executive::error::{FcpError, Result};
use crate::memory::buffer::{stage_text, BufferCaps, TAG_VAULT_READ_BUFFER};
use crate::memory::buffer_handles::BufferHandleRegistry;
use crate::memory::ephemeral::EphemeralMemory;
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct VaultReadArgs {
    /// Relative path inside the workspace vault (e.g. '10_Projects/fcp.md')
    pub relative_path: String,
}

pub struct VaultReadTool {
    pub workspace_root: PathBuf,
    pub read_limit: usize,
    pub ephemeral: Arc<EphemeralMemory>,
    pub buffer_handles: Arc<BufferHandleRegistry>,
    pub buffer_caps: BufferCaps,
    pub buffer_ttl_secs: u64,
}

#[async_trait]
impl Tool for VaultReadTool {
    fn name(&self) -> &'static str {
        "vault:read"
    }

    fn description(&self) -> &'static str {
        "Reads a file from the vault. Large files are staged as an ephemeral buffer (same chunking as web:fetch); use ephemeral:buffer_page or ephemeral:buffer_query with the short buffer_id from the receipt (e.g. buf_1)."
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

        let content = fs::read_to_string(&target_path)
            .await
            .map_err(FcpError::Io)?;

        let max_bytes = self.read_limit * 4;

        if content.len() <= max_bytes {
            return Ok(content);
        }

        let headers: Vec<&str> = content
            .lines()
            .filter(|line| line.trim_start().starts_with('#'))
            .collect();
        let map = headers.join("\n");

        let (stored, mut receipt) = stage_text(
            self.ephemeral.as_ref(),
            self.name(),
            &args.relative_path,
            &content,
            vec![TAG_VAULT_READ_BUFFER.to_string()],
            self.buffer_ttl_secs,
            &self.buffer_caps,
        )
        .await?;

        let handle = self
            .buffer_handles
            .register(stored.staged_id.clone())
            .await;
        receipt.buffer_id = handle.clone();

        let receipt_json = serde_json::to_string(&receipt).map_err(FcpError::ParseFault)?;

        Ok(format!(
            "[Large vault file staged as ephemeral buffer — full text was bounded to num_ctx-aligned caps, same as web artifacts.]\n\n[FCP_BUFFER_REF — use this exact token as buffer_id in ephemeral:buffer_page / ephemeral:buffer_query]\n{handle}\n\n{receipt_json}\n\n[FILE MAP — markdown headers from the file]\n{map}\n",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::buffer_handles::BufferHandleRegistry;
    use tempfile::tempdir;
    use tokio::fs;

    fn test_caps() -> BufferCaps {
        BufferCaps {
            max_staged_bytes: 12_000,
            chunk_target_chars: 3000,
            preview_chars: 1500,
            max_chunks: 4096,
            page_response_max_chars: 12_000,
        }
    }

    #[tokio::test]
    async fn test_vault_read_normal() -> Result<()> {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("normal.md");
        fs::write(&file_path, "Hello, Vault!")
            .await
            .expect("write normal.md");

        let tool = VaultReadTool {
            workspace_root: dir.path().to_path_buf(),
            read_limit: 3000,
            ephemeral: Arc::new(EphemeralMemory::new("ws".into())),
            buffer_handles: Arc::new(BufferHandleRegistry::new()),
            buffer_caps: test_caps(),
            buffer_ttl_secs: 60,
        };
        let args = serde_json::json!({ "relative_path": "normal.md" });

        let result = tool.execute(args).await?;
        assert_eq!(result, "Hello, Vault!");
        Ok(())
    }

    #[tokio::test]
    async fn test_vault_read_exceeds_tokens_stages_buffer() -> Result<()> {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("massive.md");

        let mut massive_content = String::from("# Header 1\n");
        massive_content.push_str(&"A".repeat(12001));
        massive_content.push_str("\n## Header 2\n");
        massive_content.push_str(&"B".repeat(10));

        fs::write(&file_path, massive_content)
            .await
            .expect("write massive.md");

        let mem = Arc::new(EphemeralMemory::new("ws".into()));
        let handles = Arc::new(BufferHandleRegistry::new());
        let tool = VaultReadTool {
            workspace_root: dir.path().to_path_buf(),
            read_limit: 3000,
            ephemeral: mem.clone(),
            buffer_handles: handles.clone(),
            buffer_caps: test_caps(),
            buffer_ttl_secs: 60,
        };
        let args = serde_json::json!({ "relative_path": "massive.md" });

        let result = tool.execute(args).await?;
        assert!(result.contains("Large vault file staged"));
        assert!(result.contains("buffer_id"));
        assert!(result.contains("# Header 1"));
        assert!(result.contains("## Header 2"));
        assert!(result.contains("ephemeral:buffer_page"));

        let json_block = result
            .split("\n\n")
            .find(|s| s.trim_start().starts_with('{'))
            .expect("receipt json block");
        let v: serde_json::Value = serde_json::from_str(json_block.trim()).expect("receipt json");
        let bid = v["buffer_id"].as_str().expect("buffer_id");
        let page_tool = crate::tools::ephemeral::EphemeralBufferPageTool {
            ephemeral: mem,
            buffer_handles: handles,
            caps: test_caps(),
        };
        let page = page_tool
            .execute(serde_json::json!({
                "buffer_id": bid,
                "page": 0,
                "page_size": 1
            }))
            .await
            .expect("page");
        assert!(page.contains("\"chunks\""));
        Ok(())
    }
}
