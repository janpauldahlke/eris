use std::sync::Arc;

use async_trait::async_trait;
use schemars::schema::RootSchema;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::executive::error::{FcpError, Result};
use crate::memory::buffer::{
    is_chunked_buffer_entry, page_chunks, parse_buffered_blob, BufferCaps,
};
use crate::memory::buffer_handles::{BufferHandleRegistry, BufferHandleResolveError};
use crate::memory::ephemeral::EphemeralMemory;
use crate::tools::context_view_hint::{ToolContextViewHint, API_TOOL_SNIPPET_CHARS};
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct EphemeralBufferPageArgs {
    /// Short handle from the receipt (e.g. `buf_1`) or legacy raw ephemeral UUID.
    pub buffer_id: String,
    /// 0-based page index.
    pub page: usize,
    /// Chunks per page (clamped 1–64). Defaults to 1.
    #[serde(default = "default_page_size")]
    pub page_size: usize,
}

fn default_page_size() -> usize {
    1
}

pub struct EphemeralBufferPageTool {
    pub ephemeral: Arc<EphemeralMemory>,
    pub buffer_handles: Arc<BufferHandleRegistry>,
    pub caps: BufferCaps,
}

#[async_trait]
impl Tool for EphemeralBufferPageTool {
    fn name(&self) -> &'static str {
        "ephemeral:buffer_page"
    }

    fn description(&self) -> &'static str {
        "Read sequential chunks from a staged large vault file or web artifact. If the user asks for detail on a specific chapter or section, page through (or combine with ephemeral:buffer_query) until you have seen the relevant chunks—do not fabricate long-form content from titles alone."
    }

    fn parameters_schema(&self) -> RootSchema {
        schemars::schema_for!(EphemeralBufferPageArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet {
            max_chars: API_TOOL_SNIPPET_CHARS,
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: EphemeralBufferPageArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        if args.buffer_id.trim().is_empty() {
            return Err(FcpError::SchemaViolation(
                "buffer_id cannot be empty".to_string(),
            ));
        }

        let staged_key = match self
            .buffer_handles
            .resolve_for_lookup(&args.buffer_id)
            .await
        {
            Ok(k) => k,
            Err(BufferHandleResolveError::Empty) => {
                return Err(FcpError::SchemaViolation(
                    "buffer_id cannot be empty".to_string(),
                ));
            }
            Err(BufferHandleResolveError::UnknownHandle) => {
                return Err(FcpError::ToolFault {
                    tool_name: self.name().to_string(),
                    reason: "Unknown buffer_id; use the buf_N token from your latest vault:read or web:fetch receipt or the [FCP BUFFER SESSION] block.".to_string(),
                });
            }
        };

        let entry = self
            .ephemeral
            .get_by_id(&staged_key)
            .await
            .ok_or_else(|| FcpError::ToolFault {
                tool_name: self.name().to_string(),
                reason: "Buffer not found or expired; re-run vault:read or web:fetch.".to_string(),
            })?;

        if !is_chunked_buffer_entry(&entry.tags) {
            return Err(FcpError::ToolFault {
                tool_name: self.name().to_string(),
                reason: "This staged_id is not a chunked buffer entry.".to_string(),
            });
        }

        let blob = parse_buffered_blob(&entry.data)?;
        let response = page_chunks(
            self.name(),
            &blob,
            args.buffer_id.trim(),
            args.page,
            args.page_size,
            self.caps.page_response_max_chars,
        )?;
        serde_json::to_string(&response).map_err(FcpError::ParseFault)
    }
}
