use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::executive::error::{FcpError, Result};
use crate::memory::document_store::DocumentStore;
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct DocDeleteArgs {
    pub doc_id: String,
}

pub struct DocDeleteTool {
    pub workspace_root: PathBuf,
    pub store: Arc<DocumentStore>,
}

#[async_trait]
impl Tool for DocDeleteTool {
    fn name(&self) -> &'static str {
        "doc:delete"
    }

    fn description(&self) -> &'static str {
        "Remove a document and all its chunks from the document store, plus its 40_MEDIA discovery card and memory index point."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(DocDeleteArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed: DocDeleteArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let count = self
            .store
            .delete_document(&self.workspace_root, &parsed.doc_id)
            .await?;
        Ok(json!({
            "ok": true,
            "doc_id": parsed.doc_id,
            "chunks_deleted": count,
        })
        .to_string())
    }
}
