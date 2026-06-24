use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::executive::error::{FcpError, Result};
use crate::memory::document_store::DocumentStore;
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct DocReadArgs {
    /// Which document to read (UUID from doc:list or doc:ingest receipt).
    pub doc_id: String,
    /// First chunk index to return (0-based). Defaults to 0.
    #[serde(default)]
    pub start: Option<u32>,
    /// How many chunks to return. Defaults to config read_page_size_default.
    #[serde(default)]
    pub count: Option<u32>,
}

pub struct DocReadTool {
    pub store: Arc<DocumentStore>,
    pub page_size_default: u32,
    pub page_size_max: u32,
}

#[async_trait]
impl Tool for DocReadTool {
    fn name(&self) -> &'static str {
        "doc:read"
    }

    fn description(&self) -> &'static str {
        "Paginated sequential reading of an ingested document's chunks. Returns ordered chunk texts with index markers. Use start/count to page through large documents."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(DocReadArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed: DocReadArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let doc_id = parsed.doc_id.trim();
        if doc_id.is_empty() {
            return Err(FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "doc_id is required".into(),
            });
        }

        let start = parsed.start.unwrap_or(0);
        let count = parsed
            .count
            .unwrap_or(self.page_size_default)
            .clamp(1, self.page_size_max.max(1));

        let chunks = self.store.read_chunks_page(doc_id, start, count).await?;
        if chunks.is_empty() {
            return Ok(format!(
                "Document {doc_id}: no chunks in range [{start}..{}). The document may have fewer chunks.",
                start + count
            ));
        }

        let total_chunks = chunks.first().map(|c| c.total_chunks).unwrap_or(0);
        let source_name = chunks
            .first()
            .map(|c| c.source_name.as_str())
            .unwrap_or("unknown");
        let last_index = chunks.last().map(|c| c.chunk_index).unwrap_or(start);

        let mut out = format!(
            "Document: {} (chunks {}-{} of {})\n\n",
            source_name, start, last_index, total_chunks
        );

        for chunk in &chunks {
            out.push_str(&format!("[chunk {}/{}]\n", chunk.chunk_index, total_chunks));
            out.push_str(chunk.text.trim());
            out.push_str("\n\n");
        }

        let remaining = total_chunks.saturating_sub(last_index + 1);
        if remaining > 0 {
            out.push_str(&format!(
                "--- {} more chunks remaining (use start={} to continue) ---\n",
                remaining,
                last_index + 1
            ));
        }

        Ok(out)
    }
}
