use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::executive::error::Result;
use crate::memory::document_store::{DocumentStore, format_list_documents_markdown};
use crate::tools::traits::Tool;

pub struct DocListTool {
    pub store: Arc<DocumentStore>,
}

#[async_trait]
impl Tool for DocListTool {
    fn name(&self) -> &'static str {
        "doc:list"
    }

    fn description(&self) -> &'static str {
        "List all ingested documents in the document RAG store (doc_id, source name, chunk count)."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(serde_json::Value)
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        let docs = self.store.list_documents().await?;
        Ok(format_list_documents_markdown(&docs))
    }
}
