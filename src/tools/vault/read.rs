use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use tokio::fs;

use crate::executive::error::{FcpError, Result};
use crate::memory::buffer::{
    parse_buffered_blob, stage_text, stage_text_replace, BufferCaps, TAG_VAULT_READ_BUFFER,
    VaultLensReceipt,
};
use crate::memory::buffer_handles::BufferHandleRegistry;
use crate::memory::ephemeral::EphemeralMemory;
use crate::tools::traits::Tool;
use crate::util::utf8_file_window::read_utf8_file_window;

#[derive(Deserialize, JsonSchema)]
pub struct VaultReadArgs {
    /// Relative path inside the workspace vault (e.g. '10_Projects/fcp.md')
    #[serde(alias = "path")]
    pub relative_path: String,
    /// Byte offset in the file where this read lens starts (large files load only a window). Use `suggested_next_byte_offset` / `suggested_prev_byte_offset` from the prior receipt's `vault_lens`. Default 0.
    #[serde(default)]
    pub byte_offset: Option<usize>,
    /// When moving the lens, pass the existing buffer handle (e.g. buf_1). Chunks are replaced in place; the same handle stays valid for follow-up `ephemeral:buffer_*` calls. Must match `relative_path` stored in that buffer.
    #[serde(default, alias = "reuse_buffer_id")]
    pub buffer_id: Option<String>,
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
        "Reads a file from the vault. Large files are loaded as a sliding **lens** (a byte window): use `byte_offset` and optional `buffer_id` to move along the file while keeping the same buffer handle. Use ephemeral:buffer_page / ephemeral:buffer_query inside the current lens; check receipt field `vault_lens` for file totals and suggested offsets."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(VaultReadArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: VaultReadArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;

        let target_path = self.workspace_root.join(&args.relative_path);
        if !target_path.starts_with(&self.workspace_root) {
            return Err(FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "Path Traversal Denied".into(),
            });
        }

        let byte_offset = args.byte_offset.unwrap_or(0);
        let relocate_handle: Option<String> = args
            .buffer_id
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let inline_max = self.read_limit.saturating_mul(4);
        let meta = fs::metadata(&target_path).await.map_err(FcpError::Io)?;
        let total_bytes = meta.len() as usize;

        let reuse_staged: Option<String> = if let Some(bid) = &relocate_handle {
            let staged = self
                .buffer_handles
                .resolve_for_lookup(bid)
                .await
                .map_err(|_| FcpError::ToolFault {
                    tool_name: self.name().into(),
                    reason: format!(
                        "Unknown buffer_id `{bid}` for lens move. Use the handle from your latest vault:read receipt (e.g. buf_1)."
                    ),
                })?;
            let entry = self
                .ephemeral
                .get_by_id(&staged)
                .await
                .ok_or_else(|| FcpError::ToolFault {
                    tool_name: self.name().into(),
                    reason: "That buffer expired or was evicted; run vault:read without buffer_id to open a fresh lens.".into(),
                })?;
            let existing_blob = parse_buffered_blob(&entry.data)?;
            if existing_blob.source != args.relative_path {
                return Err(FcpError::ToolFault {
                    tool_name: self.name().into(),
                    reason: format!(
                        "buffer_id refers to source '{}' but relative_path is '{}'; paths must match when moving the lens.",
                        existing_blob.source, args.relative_path
                    ),
                });
            }
            Some(staged)
        } else {
            None
        };

        if total_bytes <= inline_max && byte_offset == 0 && reuse_staged.is_none() {
            let content = fs::read_to_string(&target_path)
                .await
                .map_err(FcpError::Io)?;
            return Ok(content);
        }

        if total_bytes <= inline_max && byte_offset > 0 && reuse_staged.is_none() {
            let (suffix, _, _, _) =
                read_utf8_file_window(&target_path, byte_offset, inline_max)
                    .await
                    .map_err(FcpError::Io)?;
            return Ok(suffix);
        }

        let window_max = self.buffer_caps.max_staged_bytes;
        let (window_text, aligned_start, raw_end, total_check) =
            read_utf8_file_window(&target_path, byte_offset, window_max)
                .await
                .map_err(FcpError::Io)?;
        if total_check != total_bytes {
            tracing::warn!(
                file_len_meta = total_bytes,
                file_len_window = total_check,
                "file length mismatch between metadata and window read"
            );
        }

        if window_text.is_empty() {
            return Err(FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: format!(
                    "No readable text in this lens (byte_offset {byte_offset}, file bytes {total_bytes}). Try suggested_prev_byte_offset from your last receipt or byte_offset 0."
                ),
            });
        }

        let prev_off = if aligned_start > 0 {
            Some(aligned_start.saturating_sub(window_max))
        } else {
            None
        };
        let next_off = if raw_end < total_bytes {
            Some(raw_end)
        } else {
            None
        };

        let vault_lens = VaultLensReceipt {
            source_total_bytes: total_bytes,
            lens_file_byte_range: [aligned_start, raw_end],
            suggested_prev_byte_offset: prev_off,
            suggested_next_byte_offset: next_off,
        };

        let tags = vec![TAG_VAULT_READ_BUFFER.to_string()];

        let (stored, mut receipt) = if let Some(ref sid) = reuse_staged {
            stage_text_replace(
                self.ephemeral.as_ref(),
                self.name(),
                &args.relative_path,
                &window_text,
                tags,
                self.buffer_ttl_secs,
                &self.buffer_caps,
                sid,
                Some(vault_lens),
            )
            .await?
        } else {
            stage_text(
                self.ephemeral.as_ref(),
                self.name(),
                &args.relative_path,
                &window_text,
                tags,
                self.buffer_ttl_secs,
                &self.buffer_caps,
                Some(vault_lens),
            )
            .await?
        };

        let handle = if let Some(bid) = relocate_handle {
            bid
        } else {
            self.buffer_handles.register(stored.staged_id.clone()).await
        };
        receipt.buffer_id = handle.clone();

        let header_lines: Vec<&str> = window_text
            .lines()
            .filter(|line| line.trim_start().starts_with('#'))
            .take(400)
            .collect();
        let map = header_lines.join("\n");

        let receipt_json = serde_json::to_string(&receipt).map_err(FcpError::ParseFault)?;

        Ok(format!(
            "[Large vault file — lens applied to a byte window. Only text in `vault_lens` is buffered; FILE MAP lists headings inside this lens only.]\n\n[FCP_BUFFER_REF — use as buffer_id for ephemeral:buffer_*; pass back to vault:read with the same relative_path to slide the lens without changing handles]\n{handle}\n\n{receipt_json}\n\n[FILE MAP — markdown headings in this lens only]\n{map}\n",
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
        assert!(result.contains("lens applied"));
        assert!(result.contains("buffer_id"));
        assert!(result.contains("# Header 1"));
        assert!(result.contains("vault_lens"));
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

    #[tokio::test]
    async fn vault_read_relocate_keeps_handle_and_shifts_content() -> Result<()> {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("book.md");
        let marker = "XYZZY_LENS_MARKER_UNIQUE";
        let mut body = String::from("# Start\n");
        body.push_str(&"x".repeat(14_000));
        body.push_str("\n# Middle\n");
        body.push_str(marker);
        body.push_str("\n# End\n");
        body.push_str(&"y".repeat(14_000));
        fs::write(&file_path, &body).await.expect("write");

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

        let first = tool
            .execute(serde_json::json!({ "relative_path": "book.md" }))
            .await?;
        assert!(
            !first.contains(marker),
            "marker should lie beyond first lens"
        );
        let json_block = first
            .split("\n\n")
            .find(|s| s.trim_start().starts_with('{'))
            .expect("receipt");
        let v: serde_json::Value = serde_json::from_str(json_block.trim()).expect("json");
        let bid = v["buffer_id"].as_str().expect("buffer_id").to_string();
        let next_off = v["vault_lens"]["suggested_next_byte_offset"]
            .as_u64()
            .expect("next offset") as usize;

        let second = tool
            .execute(serde_json::json!({
                "relative_path": "book.md",
                "buffer_id": bid,
                "byte_offset": next_off,
            }))
            .await?;
        assert!(
            second.contains(&format!("\"buffer_id\":\"{bid}\"")),
            "handle should stay stable after in-place lens replace"
        );

        let query_tool = crate::tools::ephemeral::EphemeralBufferQueryTool {
            ephemeral: mem,
            buffer_handles: handles,
            semantic: None,
            max_snippet_chars: 200,
            max_total_chars: 800,
        };
        let q = query_tool
            .execute(serde_json::json!({
                "buffer_id": bid,
                "query": "XYZZY_LENS_MARKER",
            }))
            .await
            .expect("query");
        assert!(q.contains("XYZZY_LENS_MARKER"));
        Ok(())
    }
}
