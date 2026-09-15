pub mod embedding;
pub mod grammar;
pub mod llama_cpp;
pub mod ollama;
pub(crate) mod openai_wire;
pub mod openrouter;
pub mod router;
pub mod structured;
pub mod token_metrics;
pub mod traits;

pub use self::embedding::EmbeddingProvider;
pub use self::llama_cpp::LlamaCppClient;
pub use self::openrouter::OpenRouterClient;
pub use self::token_metrics::{
    LlmTokenSnapshot, TokenMetricsReader, channel as token_metrics_channel,
    publish as publish_llm_token_snapshot,
};
pub use self::traits::{EngineResponse, LlmEngine, LlmGenerateOptions, Message, Role};

use self::ollama::OllamaClient;
use async_trait::async_trait;
use tokio::sync::mpsc;

/// Runtime dispatch over compiled-in engine backends. Keeps `Orchestrator<E>` generic
/// without requiring `Box<dyn LlmEngine>` or touching every `impl` block.
pub enum AnyEngine {
    Ollama(OllamaClient),
    LlamaCpp(LlamaCppClient),
    OpenRouter(OpenRouterClient),
}

impl AnyEngine {
    /// Set the GBNF grammar on the inner engine (only meaningful for `LlamaCpp`;
    /// OpenRouter constrains output via `response_format` instead).
    pub fn set_grammar(&mut self, grammar: String) {
        match self {
            Self::LlamaCpp(e) => e.set_grammar(grammar),
            Self::Ollama(_) | Self::OpenRouter(_) => {}
        }
    }
}

#[async_trait]
impl LlmEngine for AnyEngine {
    async fn generate(
        &self,
        stack: &[Message],
        available_tools_json: &str,
        stream_tx: Option<mpsc::UnboundedSender<String>>,
        options: LlmGenerateOptions,
    ) -> crate::executive::error::Result<EngineResponse> {
        match self {
            Self::Ollama(e) => e.generate(stack, available_tools_json, stream_tx, options).await,
            Self::LlamaCpp(e) => e.generate(stack, available_tools_json, stream_tx, options).await,
            Self::OpenRouter(e) => e.generate(stack, available_tools_json, stream_tx, options).await,
        }
    }
}
