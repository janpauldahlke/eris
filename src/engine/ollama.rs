use std::sync::Arc;
use tokio::sync::{mpsc, watch};
use async_trait::async_trait;
use ollama_rs::Ollama;
use crate::config::AppConfig;
use crate::engine::{LlmEngine, Message, EngineResponse};
use crate::engine::token_metrics::{self, LlmTokenSnapshot};
use crate::executive::error::Result;

pub struct OllamaClient {
    pub client: Ollama,
    pub config: Arc<AppConfig>,
    /// When set, every successful `generate` publishes [`LlmTokenSnapshot`] for the last Ollama response.
    pub token_metrics_tx: Option<watch::Sender<LlmTokenSnapshot>>,
}

impl OllamaClient {
    pub fn new(client: Ollama, config: Arc<AppConfig>) -> Self {
        Self {
            client,
            config,
            token_metrics_tx: None,
        }
    }

    pub fn with_token_metrics(
        client: Ollama,
        config: Arc<AppConfig>,
        token_metrics_tx: watch::Sender<LlmTokenSnapshot>,
    ) -> Self {
        Self {
            client,
            config,
            token_metrics_tx: Some(token_metrics_tx),
        }
    }
}

#[async_trait]
impl LlmEngine for OllamaClient {
    async fn generate(
        &self,
        stack: &[Message],
        available_tools_json: &str,
        stream_tx: Option<mpsc::UnboundedSender<String>>
    ) -> Result<EngineResponse> {
        use ollama_rs::generation::chat::{ChatMessage, MessageRole};
        use ollama_rs::generation::chat::request::ChatMessageRequest;
        use tokio_stream::StreamExt;
        use std::time::Duration;
        use crate::executive::error::FcpError;
        use ollama_rs::generation::parameters::FormatType;

        let mut chat_messages = Vec::new();
        let mut injected = false;

        for msg in stack {
            let role = match msg.role.as_str() {
                "system" => MessageRole::System,
                "user" => MessageRole::User,
                "assistant" => MessageRole::Assistant,
                _ => MessageRole::User,
            };

            let mut content = msg.content.clone();
            
            if role == MessageRole::System && !injected && !available_tools_json.is_empty() {
                content = format!("{}\n\nAVAILABLE TOOLS:\n{}", content, available_tools_json);
                injected = true;
            }

            chat_messages.push(ChatMessage::new(role, content));
        }

        if !injected && !available_tools_json.is_empty() {
            chat_messages.insert(0, ChatMessage::new(
                MessageRole::System,
                format!("AVAILABLE TOOLS:\n{}", available_tools_json)
            ));
        }

        tracing::info!(
            model = %self.config.model_name,
            message_count = chat_messages.len(),
            timeout_secs = self.config.generation_timeout_secs,
            streaming = stream_tx.is_some(),
            "Sending chat request to Ollama"
        );

        use ollama_rs::generation::options::GenerationOptions;

        let gen_options = GenerationOptions::default()
            .num_ctx(self.config.num_ctx as u64);

        let request = ChatMessageRequest::new(
            self.config.model_name.clone(),
            chat_messages
        )
        .format(FormatType::Json)
        .options(gen_options);

        let timeout = Duration::from_secs(self.config.generation_timeout_secs);

        if let Some(tx) = stream_tx {
            let stream_future = self.client.send_chat_messages_stream(request);
            let mut stream = match tokio::time::timeout(timeout, stream_future).await {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => {
                    tracing::error!(error = %e, "Ollama stream connection failed");
                    return Err(FcpError::NetworkFault(e.to_string()));
                }
                Err(_) => {
                    tracing::error!(timeout_secs = self.config.generation_timeout_secs, "Ollama stream timed out on connect");
                    return Err(FcpError::EngineFault("Generation timed out".to_string()));
                }
            };

            let mut full_content = String::new();
            let mut prompt_tokens = 0;
            let mut generated_tokens = 0;

            // Stream chunks
            loop {
                let chunk_future = stream.next();
                let next_chunk = match tokio::time::timeout(timeout, chunk_future).await {
                    Ok(Some(chunk)) => chunk,
                    Ok(None) => break,
                    Err(_) => return Err(FcpError::EngineFault("Generation timed out during stream".to_string())),
                };

                match next_chunk {
                    Ok(res) => {
                        full_content.push_str(&res.message.content);
                        let _ = tx.send(res.message.content);

                        if let Some(fd) = res.final_data {
                            prompt_tokens = fd.prompt_eval_count as usize;
                            generated_tokens = fd.eval_count as usize;
                        }
                    }
                    Err(_) => return Err(FcpError::EngineFault("Stream error".to_string())),
                }
            }

            token_metrics::publish(&self.token_metrics_tx, prompt_tokens, generated_tokens);

            Ok(EngineResponse {
                content: full_content,
                prompt_tokens,
                generated_tokens,
            })
        } else {
            let future = self.client.send_chat_messages(request);
            match tokio::time::timeout(timeout, future).await {
                Ok(Ok(response)) => {
                    let content = response.message.content;
                    let (prompt_tokens, generated_tokens) = if let Some(fd) = response.final_data {
                        (fd.prompt_eval_count as usize, fd.eval_count as usize)
                    } else {
                        (0, 0)
                    };
                    tracing::info!(prompt_tokens, generated_tokens, content_len = content.len(), "Ollama non-stream response received");
                    token_metrics::publish(&self.token_metrics_tx, prompt_tokens, generated_tokens);
                    Ok(EngineResponse {
                        content,
                        prompt_tokens,
                        generated_tokens,
                    })
                }
                Ok(Err(e)) => {
                    tracing::error!(error = %e, "Ollama request failed");
                    Err(FcpError::NetworkFault(e.to_string()))
                }
                Err(_) => {
                    tracing::error!(timeout_secs = self.config.generation_timeout_secs, "Ollama request timed out");
                    Err(FcpError::EngineFault("Generation timed out".to_string()))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::{method, path};
    use crate::executive::error::FcpError;

    #[tokio::test]
    async fn test_ollama_client_offline_returns_network_fault() {
        let mut config = AppConfig::default();
        config.ollama_host = "http://localhost:65535".to_string(); // Dead port
        
        let client = Ollama::new("http://localhost".to_string(), 65535);
        let engine = OllamaClient::new(client, Arc::new(config));

        let result = engine.generate(&[], "{}", None).await;

        match result {
            Err(FcpError::NetworkFault(_)) => (),
            _ => panic!("Expected NetworkFault, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn test_ollama_client_timeout_returns_engine_fault() {
        let mock_server = MockServer::start().await;
        
        let mut config = AppConfig::default();
        config.ollama_host = mock_server.uri();
        config.generation_timeout_secs = 1;

        let parsed_url = url::Url::parse(&mock_server.uri()).unwrap();
        let client = Ollama::new(
            format!("{}://{}", parsed_url.scheme(), parsed_url.host_str().unwrap()),
            parsed_url.port().unwrap_or(80)
        );
        let engine = OllamaClient::new(client, Arc::new(config));

        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
            .mount(&mock_server)
            .await;

        let result = engine.generate(&[], "{}", None).await;

        match result {
            Err(FcpError::EngineFault(_)) => (),
            _ => panic!("Expected EngineFault, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn test_ollama_client_handles_valid_response() {
        let mock_server = MockServer::start().await;
        
        let mut config = AppConfig::default();
        config.ollama_host = mock_server.uri();

        let parsed_url = url::Url::parse(&mock_server.uri()).unwrap();
        let client = Ollama::new(
            format!("{}://{}", parsed_url.scheme(), parsed_url.host_str().unwrap()),
            parsed_url.port().unwrap_or(80)
        );
        let engine = OllamaClient::new(client, Arc::new(config));

        let mock_response = serde_json::json!({
            "model": "llama3.2",
            "created_at": "2024-01-01T00:00:00Z",
            "message": {
                "role": "assistant",
                "content": "Hello world"
            },
            "done": true,
            "total_duration": 1000,
            "load_duration": 100,
            "prompt_eval_count": 10,
            "prompt_eval_duration": 200,
            "eval_count": 5,
            "eval_duration": 300
        });

        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_response))
            .mount(&mock_server)
            .await;

        let result = engine.generate(&[], "{}", None).await.expect("Expected a valid EngineResponse");

        assert_eq!(result.content, "Hello world");
        assert_eq!(result.prompt_tokens, 10);
        assert_eq!(result.generated_tokens, 5);
    }
}
