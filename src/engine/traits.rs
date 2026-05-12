use crate::executive::error::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;

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
    /// Wall-clock ms for the completed request (streaming or non-streaming), for throughput metrics.
    pub generation_ms: u64,
}

impl Default for EngineResponse {
    fn default() -> Self {
        Self {
            content: String::new(),
            prompt_tokens: 0,
            generated_tokens: 0,
            generation_ms: 0,
        }
    }
}

#[async_trait]
pub trait LlmEngine: Send + Sync {
    async fn generate(
        &self,
        stack: &[Message],
        available_tools_json: &str,
        stream_tx: Option<mpsc::UnboundedSender<String>>,
    ) -> Result<EngineResponse>;
}
