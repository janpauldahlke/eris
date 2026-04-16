use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::engine::llama_server::LlamaServerEngine;
use crate::engine::ollama::OllamaClient;
use crate::engine::{EngineResponse, GenerationConstraints, LlmEngine, Message};
use crate::executive::error::Result;

pub enum ChatEngine {
    Ollama(OllamaClient),
    LlamaServer(LlamaServerEngine),
}

#[async_trait]
impl LlmEngine for ChatEngine {
    async fn generate(
        &self,
        stack: &[Message],
        available_tools_json: &str,
        stream_tx: Option<mpsc::UnboundedSender<String>>,
    ) -> Result<EngineResponse> {
        match self {
            Self::Ollama(engine) => engine.generate(stack, available_tools_json, stream_tx).await,
            Self::LlamaServer(engine) => {
                engine.generate(stack, available_tools_json, stream_tx).await
            }
        }
    }

    async fn generate_constrained(
        &self,
        stack: &[Message],
        available_tools_json: &str,
        constraints: &GenerationConstraints,
        stream_tx: Option<mpsc::UnboundedSender<String>>,
    ) -> Result<EngineResponse> {
        match self {
            Self::Ollama(engine) => {
                engine
                    .generate_constrained(stack, available_tools_json, constraints, stream_tx)
                    .await
            }
            Self::LlamaServer(engine) => {
                engine
                    .generate_constrained(stack, available_tools_json, constraints, stream_tx)
                    .await
            }
        }
    }
}
