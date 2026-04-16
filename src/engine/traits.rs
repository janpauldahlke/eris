use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;
use crate::executive::error::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub role: String, // "system", "user", "assistant"
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineResponse {
    pub content: String,
    pub prompt_tokens: usize,
    pub generated_tokens: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GenerationConstraints {
    /// OpenAPI/JSON Schema object to enforce during decoding.
    pub schema: Value,
    /// Stable schema identifier used by compatible backends.
    pub schema_name: String,
    /// Backend strictness hint; backends that support strict schema mode should honor this.
    pub strict: bool,
}

impl GenerationConstraints {
    pub fn new(schema: Value, schema_name: impl Into<String>) -> Self {
        Self {
            schema,
            schema_name: schema_name.into(),
            strict: true,
        }
    }
}

#[async_trait]
pub trait LlmEngine: Send + Sync {
    async fn generate(
        &self,
        stack: &[Message],
        available_tools_json: &str,
        stream_tx: Option<mpsc::UnboundedSender<String>>
    ) -> Result<EngineResponse>;

    /// Constrained generation for deterministic JSON contracts.
    ///
    /// Implementors may override this for engine-level grammar/schema enforcement.
    /// Default behavior falls back to unconstrained generation.
    async fn generate_constrained(
        &self,
        stack: &[Message],
        available_tools_json: &str,
        constraints: &GenerationConstraints,
        stream_tx: Option<mpsc::UnboundedSender<String>>,
    ) -> Result<EngineResponse> {
        let _ = constraints;
        self.generate(stack, available_tools_json, stream_tx).await
    }
}
