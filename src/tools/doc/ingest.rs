use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::executive::error::{FcpError, Result};
use crate::memory::document_ingest_queue::DocumentIngestQueue;
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct DocIngestArgs {
    /// Vault-relative path (e.g. `99_USER_UPLOADED/files/{uuid}.pdf`).
    pub relative_path: String,
    /// Optional human-friendly title override for catalog card and chunk metadata.
    #[serde(default)]
    pub source_label: Option<String>,
}

pub struct DocIngestTool {
    pub ingest_queue: Arc<DocumentIngestQueue>,
}

#[async_trait]
impl Tool for DocIngestTool {
    fn name(&self) -> &'static str {
        "doc:ingest"
    }

    fn description(&self) -> &'static str {
        "Parse, chunk, embed, and index an uploaded document (PDF/Markdown/text) into the document RAG store. Also creates a 40_MEDIA discovery card for memory recall. Re-ingests when the file at the same path changed. Ingests run one at a time on the document queue."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(DocIngestArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed: DocIngestArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let rel = parsed.relative_path.replace('\\', "/");
        if rel.trim().is_empty() {
            return Err(FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "relative_path is required".into(),
            });
        }

        let receipt = self
            .ingest_queue
            .ingest_and_wait(rel, parsed.source_label)
            .await?;

        serde_json::to_string_pretty(&receipt).map_err(FcpError::ParseFault)
    }
}
