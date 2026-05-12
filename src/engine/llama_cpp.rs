use crate::config::AppConfig;
use crate::engine::token_metrics;
use crate::engine::{EngineResponse, LlmEngine, Message};
use crate::executive::error::{FcpError, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};

use crate::engine::token_metrics::LlmTokenSnapshot;

pub struct LlamaCppClient {
    http: reqwest::Client,
    chat_url: String,
    #[allow(dead_code)]
    config: Arc<AppConfig>,
    token_metrics_tx: Option<watch::Sender<LlmTokenSnapshot>>,
    /// Shared across all `generate` calls so we do not clone multi-megabyte GBNF on every request.
    grammar: Option<Arc<String>>,
}

impl LlamaCppClient {
    pub fn new(config: Arc<AppConfig>) -> Result<Self> {
        let lc = config.validate_llamacpp_config()?;
        let chat_url = format!(
            "{}/v1/chat/completions",
            lc.chat_server_url.trim_end_matches('/')
        );
        let timeout = Duration::from_secs(config.generation_timeout_secs);
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| FcpError::NetworkFault(format!("HTTP client build: {e}")))?;
        Ok(Self {
            http,
            chat_url,
            config,
            token_metrics_tx: None,
            grammar: None,
        })
    }

    pub fn with_token_metrics(mut self, tx: watch::Sender<LlmTokenSnapshot>) -> Self {
        self.token_metrics_tx = Some(tx);
        self
    }

    /// Set the GBNF grammar that constrains every subsequent `generate` call.
    pub fn set_grammar(&mut self, grammar: String) {
        self.grammar = Some(Arc::new(grammar));
    }
}

#[derive(Serialize)]
struct ChatCompletionRequest<'a> {
    messages: Vec<ChatMsg>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    n_predict: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    grammar: Option<&'a str>,
    /// Forwarded to the Jinja chat template inside llama-server.
    /// `{"enable_thinking": false}` suppresses Qwen3 `<think>` tokens so the
    /// GBNF grammar can constrain output from token 0.
    #[serde(skip_serializing_if = "Option::is_none")]
    chat_template_kwargs: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct ChatMsg {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Choice {
    message: Option<MessageContent>,
    delta: Option<DeltaContent>,
}

#[derive(Deserialize)]
struct MessageContent {
    content: Option<String>,
}

#[derive(Deserialize)]
struct DeltaContent {
    content: Option<String>,
}

#[derive(Deserialize)]
struct Usage {
    prompt_tokens: Option<usize>,
    completion_tokens: Option<usize>,
}

/// Normalize messages for chat templates that require all system content at
/// the beginning (e.g. Qwen).  Merge leading consecutive system messages into
/// one; re-role any later system messages as "user" so the wire payload never
/// violates the "system-only-at-start" invariant.
fn normalize_system_messages(messages: Vec<ChatMsg>) -> Vec<ChatMsg> {
    if messages.is_empty() {
        return messages;
    }

    let leading_system_count = messages
        .iter()
        .take_while(|m| m.role == "system")
        .count();

    let mut out = Vec::with_capacity(messages.len());

    if leading_system_count > 1 {
        let merged: String = messages[..leading_system_count]
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");
        out.push(ChatMsg {
            role: "system".to_string(),
            content: merged,
        });
    } else if leading_system_count == 1 {
        out.push(ChatMsg {
            role: messages[0].role.clone(),
            content: messages[0].content.clone(),
        });
    }

    let mut had_stray = false;
    for m in messages.into_iter().skip(leading_system_count) {
        if m.role == "system" {
            had_stray = true;
            out.push(ChatMsg {
                role: "user".to_string(),
                content: format!("[System] {}", m.content),
            });
        } else {
            out.push(m);
        }
    }

    if had_stray {
        tracing::warn!(
            "llama_cpp: stray system messages after non-system rows re-roled as user for strict chat template"
        );
    }

    out
}

async fn stream_sse_response(
    response: reqwest::Response,
    stream_tx: &mpsc::UnboundedSender<String>,
) -> Result<(String, usize, usize)> {
    use futures::StreamExt;

    let mut full_content = String::new();
    let mut prompt_tokens: usize = 0;
    let mut completion_tokens: usize = 0;

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result
            .map_err(|e| FcpError::NetworkFault(format!("llama-server stream read: {e}")))?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].trim().to_string();
            buffer = buffer[newline_pos + 1..].to_string();

            if line.is_empty() {
                continue;
            }

            let data = if let Some(stripped) = line.strip_prefix("data: ") {
                stripped.trim()
            } else {
                continue;
            };

            if data == "[DONE]" {
                return Ok((full_content, prompt_tokens, completion_tokens));
            }

            let parsed: ChatCompletionResponse = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            if let Some(usage) = &parsed.usage {
                prompt_tokens = usage.prompt_tokens.unwrap_or(0);
                completion_tokens = usage.completion_tokens.unwrap_or(0);
            }

            if let Some(choice) = parsed.choices.first() {
                if let Some(delta) = &choice.delta {
                    if let Some(content) = &delta.content {
                        full_content.push_str(content);
                        let _ = stream_tx.send(content.clone());
                    }
                }
            }
        }
    }

    Ok((full_content, prompt_tokens, completion_tokens))
}

#[async_trait]
impl LlmEngine for LlamaCppClient {
    async fn generate(
        &self,
        stack: &[Message],
        _available_tools_json: &str,
        stream_tx: Option<mpsc::UnboundedSender<String>>,
    ) -> Result<EngineResponse> {
        let raw_messages: Vec<ChatMsg> = stack
            .iter()
            .map(|m| ChatMsg {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();
        let messages = normalize_system_messages(raw_messages);

        let use_stream = stream_tx.is_some();
        let message_count = messages.len();

        let chat_template_kwargs = if self.grammar.is_some()
            && !self.config.enable_reasoning_fsm
        {
            Some(serde_json::json!({ "enable_thinking": false }))
        } else {
            None
        };

        let request_body = ChatCompletionRequest {
            messages,
            stream: use_stream,
            temperature: Some(0.7),
            n_predict: Some(-1),
            grammar: self.grammar.as_ref().map(|s| s.as_str()),
            chat_template_kwargs,
        };

        let model_label = self
            .config
            .llama_cpp
            .as_ref()
            .and_then(|lc| lc.chat_model_path.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("unknown.gguf");
        let gen_started = Instant::now();

        tracing::info!(
            engine = "llamacpp",
            model = %model_label,
            message_count,
            timeout_secs = self.config.generation_timeout_secs,
            streaming = use_stream,
            "Sending chat request to llama-server"
        );

        let response = self
            .http
            .post(&self.chat_url)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    FcpError::NetworkFault("llama-server request timed out".into())
                } else if e.is_connect() {
                    FcpError::NetworkFault(format!(
                        "llama-server connection refused at {} — is it running?",
                        self.chat_url
                    ))
                } else {
                    FcpError::NetworkFault(format!("llama-server request failed: {e}"))
                }
            })?;

        let status = response.status();
        if !status.is_success() {
            let body_excerpt = response
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(500)
                .collect::<String>();
            return Err(FcpError::NetworkFault(format!(
                "llama-server returned HTTP {status}: {body_excerpt}"
            )));
        }

        let (content, prompt_tokens, generated_tokens) = if let Some(tx) = stream_tx {
            stream_sse_response(response, &tx).await?
        } else {
            let body = response.text().await.map_err(|e| {
                FcpError::NetworkFault(format!("llama-server response read failed: {e}"))
            })?;
            let parsed: ChatCompletionResponse = serde_json::from_str(&body)?;
            let content = parsed
                .choices
                .first()
                .and_then(|c| c.message.as_ref())
                .and_then(|m| m.content.clone())
                .unwrap_or_default();
            let pt = parsed
                .usage
                .as_ref()
                .and_then(|u| u.prompt_tokens)
                .unwrap_or(0);
            let ct = parsed
                .usage
                .as_ref()
                .and_then(|u| u.completion_tokens)
                .unwrap_or(0);
            (content, pt, ct)
        };

        let generation_ms = gen_started.elapsed().as_millis() as u64;
        token_metrics::publish(
            &self.token_metrics_tx,
            prompt_tokens,
            generated_tokens,
            generation_ms,
        );

        tracing::info!(
            engine = "llamacpp",
            model = %model_label,
            prompt_tokens,
            completion_tokens = generated_tokens,
            generation_ms,
            content_len = content.len(),
            "llama-server chat response complete"
        );

        Ok(EngineResponse {
            content,
            prompt_tokens,
            generated_tokens,
            generation_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{LlamaCppConfig, LlmBackend};
    use std::path::PathBuf;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_config_with_url(url: &str, home: PathBuf) -> Arc<AppConfig> {
        let mut config = AppConfig::default();
        config.llm_backend = LlmBackend::LlamaCpp;
        config.llama_cpp = Some(LlamaCppConfig {
            home,
            chat_server_url: url.to_string(),
            embed_server_url: "http://127.0.0.1:8091".into(),
            chat_model_path: PathBuf::from("/fake/chat.gguf"),
            embed_model_path: PathBuf::from("/fake/embed.gguf"),
            n_gpu_layers: 0,
            ready_timeout_secs: 30,
        });
        config.generation_timeout_secs = 5;
        Arc::new(config)
    }

    fn make_client_from_mock(mock_url: &str) -> LlamaCppClient {
        let chat_url = format!("{}/v1/chat/completions", mock_url);
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("http client");
        LlamaCppClient {
            http,
            chat_url,
            config: Arc::new(AppConfig::default()),
            token_metrics_tx: None,
            grammar: None,
        }
    }

    #[tokio::test]
    async fn non_streaming_valid_response() {
        let mock_server = MockServer::start().await;
        let body = serde_json::json!({
            "choices": [{"message": {"content": "Hello, world!"}}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5}
        });
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&mock_server)
            .await;

        let client = make_client_from_mock(&mock_server.uri());
        let stack = vec![Message {
            role: "user".into(),
            content: "Hi".into(),
        }];
        let result = client.generate(&stack, "", None).await.expect("generate");
        assert_eq!(result.content, "Hello, world!");
        assert_eq!(result.prompt_tokens, 10);
        assert_eq!(result.generated_tokens, 5);
    }

    #[tokio::test]
    async fn token_metrics_publish_llamacpp() {
        let mock_server = MockServer::start().await;
        let body = serde_json::json!({
            "choices": [{"message": {"content": "done"}}],
            "usage": {"prompt_tokens": 42, "completion_tokens": 7}
        });
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&mock_server)
            .await;

        let (tx, rx) = token_metrics::channel();
        let reader = token_metrics::TokenMetricsReader::new(rx);
        let chat_url = format!("{}/v1/chat/completions", mock_server.uri());
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("http client");
        let client = LlamaCppClient {
            http,
            chat_url,
            config: Arc::new(AppConfig::default()),
            token_metrics_tx: Some(tx),
            grammar: None,
        };
        let stack = vec![Message {
            role: "user".into(),
            content: "Hi".into(),
        }];
        client.generate(&stack, "", None).await.expect("generate");
        let snap = reader.snapshot();
        assert_eq!(snap.prompt_tokens, 42);
        assert_eq!(snap.generated_tokens, 7);
    }

    #[tokio::test]
    async fn streaming_valid_response() {
        let mock_server = MockServer::start().await;
        let sse_body = "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n\
                        data: {\"choices\":[{\"delta\":{\"content\":\" world\"}}],\"usage\":{\"prompt_tokens\":8,\"completion_tokens\":2}}\n\n\
                        data: [DONE]\n\n";
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse_body))
            .mount(&mock_server)
            .await;

        let client = make_client_from_mock(&mock_server.uri());
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let stack = vec![Message {
            role: "user".into(),
            content: "Hi".into(),
        }];
        let result = client
            .generate(&stack, "", Some(tx))
            .await
            .expect("generate");
        assert_eq!(result.content, "Hello world");
        assert_eq!(result.prompt_tokens, 8);
        assert_eq!(result.generated_tokens, 2);

        let mut deltas = Vec::new();
        while let Ok(d) = rx.try_recv() {
            deltas.push(d);
        }
        assert_eq!(deltas, vec!["Hello", " world"]);
    }

    #[tokio::test]
    async fn streaming_forwards_deltas_to_tx() {
        let mock_server = MockServer::start().await;
        let sse_body = "data: {\"choices\":[{\"delta\":{\"content\":\"A\"}}]}\n\n\
                        data: {\"choices\":[{\"delta\":{\"content\":\"B\"}}]}\n\n\
                        data: {\"choices\":[{\"delta\":{\"content\":\"C\"}}]}\n\n\
                        data: [DONE]\n\n";
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse_body))
            .mount(&mock_server)
            .await;

        let client = make_client_from_mock(&mock_server.uri());
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let stack = vec![Message {
            role: "user".into(),
            content: "test".into(),
        }];
        client
            .generate(&stack, "", Some(tx))
            .await
            .expect("generate");

        let mut deltas = Vec::new();
        while let Ok(d) = rx.try_recv() {
            deltas.push(d);
        }
        assert_eq!(deltas, vec!["A", "B", "C"]);
    }

    #[tokio::test]
    async fn http_timeout_returns_network_fault() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
            .mount(&mock_server)
            .await;

        let chat_url = format!("{}/v1/chat/completions", mock_server.uri());
        let http = reqwest::Client::builder()
            .timeout(Duration::from_millis(100))
            .build()
            .expect("http client");
        let client = LlamaCppClient {
            http,
            chat_url,
            config: Arc::new(AppConfig::default()),
            token_metrics_tx: None,
            grammar: None,
        };
        let stack = vec![Message {
            role: "user".into(),
            content: "Hi".into(),
        }];
        let err = client.generate(&stack, "", None).await.unwrap_err();
        assert!(err.to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn http_500_returns_network_fault() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
            .mount(&mock_server)
            .await;

        let client = make_client_from_mock(&mock_server.uri());
        let stack = vec![Message {
            role: "user".into(),
            content: "Hi".into(),
        }];
        let err = client.generate(&stack, "", None).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("500"));
        assert!(msg.contains("internal error"));
    }

    #[tokio::test]
    async fn connection_refused_returns_network_fault() {
        let chat_url = "http://127.0.0.1:19999/v1/chat/completions".to_string();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .expect("http client");
        let client = LlamaCppClient {
            http,
            chat_url,
            config: Arc::new(AppConfig::default()),
            token_metrics_tx: None,
            grammar: None,
        };
        let stack = vec![Message {
            role: "user".into(),
            content: "Hi".into(),
        }];
        let err = client.generate(&stack, "", None).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("connection refused") || msg.contains("request failed"));
    }

    #[tokio::test]
    async fn missing_usage_defaults_to_zero() {
        let mock_server = MockServer::start().await;
        let body = serde_json::json!({
            "choices": [{"message": {"content": "response"}}]
        });
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&mock_server)
            .await;

        let client = make_client_from_mock(&mock_server.uri());
        let stack = vec![Message {
            role: "user".into(),
            content: "Hi".into(),
        }];
        let result = client.generate(&stack, "", None).await.expect("generate");
        assert_eq!(result.prompt_tokens, 0);
        assert_eq!(result.generated_tokens, 0);
    }

    #[tokio::test]
    async fn empty_content_in_delta_skipped() {
        let mock_server = MockServer::start().await;
        let sse_body = "data: {\"choices\":[{\"delta\":{\"content\":null}}]}\n\n\
                        data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n\
                        data: [DONE]\n\n";
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse_body))
            .mount(&mock_server)
            .await;

        let client = make_client_from_mock(&mock_server.uri());
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let stack = vec![Message {
            role: "user".into(),
            content: "test".into(),
        }];
        let result = client
            .generate(&stack, "", Some(tx))
            .await
            .expect("generate");
        assert_eq!(result.content, "ok");

        let mut deltas = Vec::new();
        while let Ok(d) = rx.try_recv() {
            deltas.push(d);
        }
        assert_eq!(deltas, vec!["ok"]);
    }

    #[tokio::test]
    async fn done_sentinel_terminates_stream() {
        let mock_server = MockServer::start().await;
        let sse_body = "data: {\"choices\":[{\"delta\":{\"content\":\"first\"}}]}\n\n\
                        data: [DONE]\n\n\
                        data: {\"choices\":[{\"delta\":{\"content\":\"SHOULD_NOT_APPEAR\"}}]}\n\n";
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse_body))
            .mount(&mock_server)
            .await;

        let client = make_client_from_mock(&mock_server.uri());
        let (tx, _rx) = mpsc::unbounded_channel::<String>();
        let stack = vec![Message {
            role: "user".into(),
            content: "test".into(),
        }];
        let result = client
            .generate(&stack, "", Some(tx))
            .await
            .expect("generate");
        assert_eq!(result.content, "first");
        assert!(!result.content.contains("SHOULD_NOT_APPEAR"));
    }

    #[tokio::test]
    async fn constructor_validates_config() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let bin_dir = tmp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("mkdir");
        std::fs::write(bin_dir.join("llama-server"), b"fake").expect("write");
        let chat_gguf = tmp.path().join("chat.gguf");
        let embed_gguf = tmp.path().join("embed.gguf");
        std::fs::write(&chat_gguf, b"fake").expect("write");
        std::fs::write(&embed_gguf, b"fake").expect("write");

        let config = test_config_with_url("http://127.0.0.1:8090", tmp.path().to_path_buf());
        let mut cfg = (*config).clone();
        cfg.llama_cpp.as_mut().expect("lc").chat_model_path = chat_gguf;
        cfg.llama_cpp.as_mut().expect("lc").embed_model_path = embed_gguf;
        let config = Arc::new(cfg);
        let result = LlamaCppClient::new(config);
        assert!(result.is_ok());

        let bad_config = Arc::new({
            let mut c = AppConfig::default();
            c.llm_backend = LlmBackend::LlamaCpp;
            c.llama_cpp = None;
            c
        });
        let result = LlamaCppClient::new(bad_config);
        assert!(result.is_err());
    }

    #[test]
    fn chat_template_kwargs_serialized_when_grammar_and_reasoning_disabled() {
        let req = ChatCompletionRequest {
            messages: vec![ChatMsg {
                role: "user".into(),
                content: "hi".into(),
            }],
            stream: false,
            temperature: Some(0.7),
            n_predict: Some(-1),
            grammar: Some("root ::= \"{}\"".into()),
            chat_template_kwargs: Some(serde_json::json!({ "enable_thinking": false })),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        let kwargs = &json["chat_template_kwargs"];
        assert_eq!(kwargs["enable_thinking"], false);
    }

    #[test]
    fn chat_template_kwargs_omitted_when_none() {
        let req = ChatCompletionRequest {
            messages: vec![],
            stream: false,
            temperature: None,
            n_predict: None,
            grammar: None,
            chat_template_kwargs: None,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert!(json.get("chat_template_kwargs").is_none());
    }

    mod normalize_system_messages_tests {
        use super::super::{ChatMsg, normalize_system_messages};

        fn sys(s: &str) -> ChatMsg {
            ChatMsg {
                role: "system".into(),
                content: s.into(),
            }
        }
        fn user(s: &str) -> ChatMsg {
            ChatMsg {
                role: "user".into(),
                content: s.into(),
            }
        }
        fn asst(s: &str) -> ChatMsg {
            ChatMsg {
                role: "assistant".into(),
                content: s.into(),
            }
        }

        #[test]
        fn empty_stack_unchanged() {
            let out = normalize_system_messages(vec![]);
            assert!(out.is_empty());
        }

        #[test]
        fn single_system_at_front_unchanged() {
            let out = normalize_system_messages(vec![sys("prompt"), user("hi")]);
            assert_eq!(out.len(), 2);
            assert_eq!(out[0].role, "system");
            assert_eq!(out[0].content, "prompt");
            assert_eq!(out[1].role, "user");
        }

        #[test]
        fn multiple_leading_systems_merged() {
            let out = normalize_system_messages(vec![
                sys("main"),
                sys("rolling summary"),
                user("hi"),
            ]);
            assert_eq!(out.len(), 2);
            assert_eq!(out[0].role, "system");
            assert!(out[0].content.contains("main"));
            assert!(out[0].content.contains("rolling summary"));
            assert_eq!(out[1].role, "user");
        }

        #[test]
        fn stray_system_after_user_reroled() {
            let out = normalize_system_messages(vec![
                sys("prompt"),
                user("hello"),
                asst("hi back"),
                sys("Tool 'x:y' succeeded: data"),
            ]);
            assert_eq!(out.len(), 4);
            assert_eq!(out[0].role, "system");
            assert_eq!(out[3].role, "user");
            assert!(out[3].content.starts_with("[System]"));
            assert!(out[3].content.contains("Tool 'x:y' succeeded: data"));
        }

        #[test]
        fn realistic_tool_turn_stack() {
            let out = normalize_system_messages(vec![
                sys("prompt"),
                user("weather?"),
                asst("{tool_calls: ...}"),
                sys("Tool 'weather:get' succeeded: 25°C"),
                sys("POST_TOOL_GUIDANCE"),
                sys("JIT guidance"),
            ]);
            assert_eq!(out[0].role, "system");
            assert_eq!(out[0].content, "prompt");
            for m in &out[1..] {
                assert_ne!(m.role, "system", "no system messages after index 0");
            }
            assert_eq!(out[3].role, "user");
            assert!(out[3].content.contains("weather:get"));
        }

        #[test]
        fn no_system_messages_at_all() {
            let out = normalize_system_messages(vec![user("hi"), asst("hello")]);
            assert_eq!(out.len(), 2);
            assert_eq!(out[0].role, "user");
            assert_eq!(out[1].role, "assistant");
        }
    }
}
