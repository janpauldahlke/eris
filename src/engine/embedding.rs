use crate::executive::error::{FcpError, Result};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Generate an embedding vector for a single text input.
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;

    /// Embedding vector dimensionality (used for Qdrant collection validation at startup).
    fn dimensions(&self) -> usize;
}

// ── OllamaEmbedding ──

use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use ollama_rs::Ollama;

pub struct OllamaEmbedding {
    ollama: Arc<Ollama>,
    model: String,
    dimensions: usize,
}

impl OllamaEmbedding {
    pub fn new(ollama: Arc<Ollama>, model: String) -> Self {
        // nomic-embed-text produces 768-dimensional vectors
        Self {
            ollama,
            model,
            dimensions: 768,
        }
    }
}

#[async_trait]
impl EmbeddingProvider for OllamaEmbedding {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let request =
            GenerateEmbeddingsRequest::new(self.model.clone(), text.to_string().into());
        tracing::debug!(
            engine = "ollama",
            model = %self.model,
            input_len = text.len(),
            "Sending embedding request to Ollama"
        );
        let response = self
            .ollama
            .generate_embeddings(request)
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;
        response
            .embeddings
            .into_iter()
            .next()
            .ok_or_else(|| FcpError::NetworkFault("Ollama returned empty embeddings".into()))
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }
}

// ── LlamaCppEmbedding ──

use serde::Deserialize;

pub struct LlamaCppEmbedding {
    http: reqwest::Client,
    embed_url: String,
    dimensions: usize,
}

impl LlamaCppEmbedding {
    pub fn new(embed_server_url: &str, timeout_secs: u64) -> Result<Self> {
        let embed_url = format!(
            "{}/v1/embeddings",
            embed_server_url.trim_end_matches('/')
        );
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .map_err(|e| FcpError::NetworkFault(format!("embed HTTP client: {e}")))?;
        // nomic-embed-text GGUF also produces 768 dims; validated at startup in Phase 6
        Ok(Self {
            http,
            embed_url,
            dimensions: 768,
        })
    }
}

#[async_trait]
impl EmbeddingProvider for LlamaCppEmbedding {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let body = serde_json::json!({ "input": text });
        tracing::debug!(
            engine = "llamacpp",
            model = %self.embed_url,
            input_len = text.len(),
            "Sending embedding request to llama-server"
        );
        let resp = self
            .http
            .post(&self.embed_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| FcpError::NetworkFault(format!("embed request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(FcpError::NetworkFault(format!(
                "embed server returned {status}: {body_text}"
            )));
        }

        let parsed: EmbeddingResponse = resp
            .json()
            .await
            .map_err(|e| FcpError::NetworkFault(format!("embed response parse: {e}")))?;

        parsed
            .data
            .into_iter()
            .next()
            .map(|d| d.embedding)
            .ok_or_else(|| FcpError::NetworkFault("embed server returned empty data".into()))
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn llamacpp_embedding_valid_response() {
        let mock_server = MockServer::start().await;
        let body = serde_json::json!({
            "data": [{"embedding": [0.1, 0.2, 0.3]}]
        });
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&mock_server)
            .await;

        let provider = LlamaCppEmbedding::new(&mock_server.uri(), 5).unwrap();
        let vec = provider.embed("hello").await.unwrap();
        assert_eq!(vec.len(), 3);
        assert!((vec[0] - 0.1).abs() < 1e-6);
    }

    #[tokio::test]
    async fn llamacpp_embedding_server_error() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
            .mount(&mock_server)
            .await;

        let provider = LlamaCppEmbedding::new(&mock_server.uri(), 5).unwrap();
        let err = provider.embed("hello").await.unwrap_err();
        assert!(err.to_string().contains("500"));
    }

    #[tokio::test]
    async fn llamacpp_embedding_empty_data() {
        let mock_server = MockServer::start().await;
        let body = serde_json::json!({ "data": [] });
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&mock_server)
            .await;

        let provider = LlamaCppEmbedding::new(&mock_server.uri(), 5).unwrap();
        let err = provider.embed("hello").await.unwrap_err();
        assert!(err.to_string().contains("empty data"));
    }

    #[test]
    fn dimensions_returns_expected() {
        let ollama_client = Arc::new(Ollama::new("http://localhost".to_string(), 11434));
        let ollama_embed = OllamaEmbedding::new(ollama_client, "nomic-embed-text".into());
        assert_eq!(ollama_embed.dimensions(), 768);

        let llama_embed = LlamaCppEmbedding::new("http://127.0.0.1:8091", 5).unwrap();
        assert_eq!(llama_embed.dimensions(), 768);
    }
}
