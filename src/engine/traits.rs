use crate::executive::error::Result;
use async_trait::async_trait;
use std::sync::Arc;
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

/// Optional knobs for a single [`LlmEngine::generate`] call (backends ignore unsupported fields).
#[derive(Debug, Clone, PartialEq)]
pub struct LlmGenerateOptions {
    /// When `Some`, overrides the backend default sampling temperature for this request only.
    pub temperature: Option<f32>,
    /// When `Some`, llama.cpp attaches this GBNF instead of the session grammar from [`crate::engine::llama_cpp::LlamaCppClient::set_grammar`].
    /// Ollama and other backends ignore this field.
    pub grammar_override: Option<Arc<str>>,
    /// When `false`, llama.cpp omits the `grammar` field from the HTTP request unless [`Self::grammar_override`] is set.
    /// Used for internal summarization passes that are not FCP agent JSON.
    pub attach_session_grammar: bool,
}

impl Default for LlmGenerateOptions {
    fn default() -> Self {
        Self {
            temperature: None,
            grammar_override: None,
            attach_session_grammar: true,
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
        options: LlmGenerateOptions,
    ) -> Result<EngineResponse>;
}
