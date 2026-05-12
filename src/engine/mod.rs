pub mod embedding;
pub mod grammar;
pub mod llama_cpp;
pub mod ollama;
pub mod router;
pub mod token_metrics;
pub mod traits;

pub use self::embedding::EmbeddingProvider;
pub use self::llama_cpp::LlamaCppClient;
pub use self::token_metrics::{
    LlmTokenSnapshot, TokenMetricsReader, channel as token_metrics_channel,
    publish as publish_llm_token_snapshot,
};
pub use self::traits::{EngineResponse, LlmEngine, Message};

use self::ollama::OllamaClient;
use async_trait::async_trait;
use tokio::sync::mpsc;

/// Runtime dispatch over compiled-in engine backends. Keeps `Orchestrator<E>` generic
/// without requiring `Box<dyn LlmEngine>` or touching every `impl` block.
pub enum AnyEngine {
    Ollama(OllamaClient),
    LlamaCpp(LlamaCppClient),
}

impl AnyEngine {
    /// Set the GBNF grammar on the inner engine (only meaningful for `LlamaCpp`).
    pub fn set_grammar(&mut self, grammar: String) {
        match self {
            Self::LlamaCpp(e) => e.set_grammar(grammar),
            Self::Ollama(_) => {}
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
    ) -> crate::executive::error::Result<EngineResponse> {
        match self {
            Self::Ollama(e) => e.generate(stack, available_tools_json, stream_tx).await,
            Self::LlamaCpp(e) => e.generate(stack, available_tools_json, stream_tx).await,
        }
    }
}
