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
