use crate::config::AppConfig;
use crate::engine::openai_wire::{to_wire_messages, ChatMsg};
use crate::engine::token_metrics;
use crate::engine::{EngineResponse, LlmEngine, LlmGenerateOptions, Message};
use crate::executive::error::{FcpError, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
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

/// Fingerprint of the active GBNF string for log correlation (not cryptographic).
fn grammar_stable_id(grammar: &str) -> u64 {
    let mut h = DefaultHasher::new();
    grammar.hash(&mut h);
    h.finish()
}

/// When [`AppConfig::enable_reasoning_fsm`] is `false` (default), forward `enable_thinking: false` into
/// llama-server’s Jinja chat template so Qwen3-style models omit `<think>…` from the assistant
/// prefix—matching [`crate::engine::ollama::OllamaClient`]'s `.think(false)` and keeping `message.content`
/// usable for JSON / GBNF from the first token. When `true`, kwargs are omitted so the template may enable
/// thinking (operators often pair with `llama-server --reasoning on` on recent builds).
fn chat_template_kwargs_for_reasoning_config(enable_reasoning_fsm: bool) -> Option<serde_json::Value> {
    if enable_reasoning_fsm {
        None
    } else {
        Some(serde_json::json!({ "enable_thinking": false }))
    }
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
    /// Forwarded to the Jinja chat template inside llama-server when
    /// [`AppConfig::enable_reasoning_fsm`] is `false` (`{"enable_thinking": false}` for Qwen3 templates).
    #[serde(skip_serializing_if = "Option::is_none")]
    chat_template_kwargs: Option<serde_json::Value>,
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
        options: LlmGenerateOptions,
    ) -> Result<EngineResponse> {
        // Shared OpenAI-wire projection (normalize + coalesce) — same path as OpenRouter.
        let messages = to_wire_messages(stack);

        let use_stream = stream_tx.is_some();
        let message_count = messages.len();

        let chat_template_kwargs =
            chat_template_kwargs_for_reasoning_config(self.config.enable_reasoning_fsm);

        let temperature = options.temperature.unwrap_or(0.7f32);

        let wire_grammar: Option<&str> = if !options.attach_session_grammar {
            options.grammar_override.as_deref()
        } else if let Some(ref o) = options.grammar_override {
            Some(o.as_ref())
        } else {
            self.grammar.as_deref().map(|s| s.as_str())
        };

        let grammar_source: &'static str = if !options.attach_session_grammar {
            if wire_grammar.is_some() {
                "override_only"
            } else {
                "none"
            }
        } else if options.grammar_override.is_some() {
            "subset_override"
        } else if self.grammar.is_some() {
            "session"
        } else {
            "none"
        };

        let (grammar_attached, grammar_len, grammar_stable_id_opt) = match wire_grammar {
            Some(g) => (true, Some(g.len()), Some(grammar_stable_id(g))),
            None => (false, None, None),
        };

        // Bounded completion: an unbounded generation that runs into the server's
        // `--ctx-size` is cut off mid-envelope, producing invalid JSON the grammar
        // cannot prevent. `<= 0` in config restores the legacy unbounded behavior.
        let n_predict = self
            .config
            .llama_cpp
            .as_ref()
            .map(|lc| lc.n_predict_max)
            .filter(|&cap| cap > 0)
            .unwrap_or(-1);

        let request_body = ChatCompletionRequest {
            messages,
            stream: use_stream,
            temperature: Some(temperature),
            n_predict: Some(n_predict),
            grammar: wire_grammar,
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
            temperature,
            enable_reasoning_fsm = self.config.enable_reasoning_fsm,
            grammar_attached,
            grammar_len,
            grammar_stable_id = grammar_stable_id_opt,
            grammar_source,
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
    use crate::engine::Role;
    use std::path::PathBuf;
    use tracing_test::traced_test;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_config_with_url(url: &str, home: PathBuf) -> Arc<AppConfig> {
        let mut config = AppConfig::default();
        config.llm_backend = LlmBackend::LlamaCpp;
        config.llama_cpp = Some(LlamaCppConfig {
            home,
            chat_server_url: url.to_string(),
            chat_model_path: PathBuf::from("/fake/chat.gguf"),
            embed_model_path: PathBuf::from("/fake/embed.gguf"),
            ..Default::default()
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

    #[tokio::test(flavor = "current_thread")]
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
            role: Role::User,
            content: "Hi".into(),
        }];
        let result = client.generate(&stack, "", None, LlmGenerateOptions::default()).await.expect("generate");
        assert_eq!(result.content, "Hello, world!");
        assert_eq!(result.prompt_tokens, 10);
        assert_eq!(result.generated_tokens, 5);
    }

    #[tokio::test(flavor = "current_thread")]
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
            role: Role::User,
            content: "Hi".into(),
        }];
        client.generate(&stack, "", None, LlmGenerateOptions::default()).await.expect("generate");
        let snap = reader.snapshot();
        assert_eq!(snap.prompt_tokens, 42);
        assert_eq!(snap.generated_tokens, 7);
    }

    #[tokio::test(flavor = "current_thread")]
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
            role: Role::User,
            content: "Hi".into(),
        }];
        let result = client
            .generate(&stack, "", Some(tx), LlmGenerateOptions::default())
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

    #[tokio::test(flavor = "current_thread")]
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
            role: Role::User,
            content: "test".into(),
        }];
        client
            .generate(&stack, "", Some(tx), LlmGenerateOptions::default())
            .await
            .expect("generate");

        let mut deltas = Vec::new();
        while let Ok(d) = rx.try_recv() {
            deltas.push(d);
        }
        assert_eq!(deltas, vec!["A", "B", "C"]);
    }

    #[tokio::test(flavor = "current_thread")]
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
            role: Role::User,
            content: "Hi".into(),
        }];
        let err = client.generate(&stack, "", None, LlmGenerateOptions::default()).await.unwrap_err();
        assert!(err.to_string().contains("timed out"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn http_500_returns_network_fault() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
            .mount(&mock_server)
            .await;

        let client = make_client_from_mock(&mock_server.uri());
        let stack = vec![Message {
            role: Role::User,
            content: "Hi".into(),
        }];
        let err = client.generate(&stack, "", None, LlmGenerateOptions::default()).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("500"));
        assert!(msg.contains("internal error"));
    }

    #[tokio::test(flavor = "current_thread")]
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
            role: Role::User,
            content: "Hi".into(),
        }];
        let err = client.generate(&stack, "", None, LlmGenerateOptions::default()).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("connection refused") || msg.contains("request failed"));
    }

    #[tokio::test(flavor = "current_thread")]
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
            role: Role::User,
            content: "Hi".into(),
        }];
        let result = client.generate(&stack, "", None, LlmGenerateOptions::default()).await.expect("generate");
        assert_eq!(result.prompt_tokens, 0);
        assert_eq!(result.generated_tokens, 0);
    }

    #[tokio::test(flavor = "current_thread")]
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
            role: Role::User,
            content: "test".into(),
        }];
        let result = client
            .generate(&stack, "", Some(tx), LlmGenerateOptions::default())
            .await
            .expect("generate");
        assert_eq!(result.content, "ok");

        let mut deltas = Vec::new();
        while let Ok(d) = rx.try_recv() {
            deltas.push(d);
        }
        assert_eq!(deltas, vec!["ok"]);
    }

    #[tokio::test(flavor = "current_thread")]
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
            role: Role::User,
            content: "test".into(),
        }];
        let result = client
            .generate(&stack, "", Some(tx), LlmGenerateOptions::default())
            .await
            .expect("generate");
        assert_eq!(result.content, "first");
        assert!(!result.content.contains("SHOULD_NOT_APPEAR"));
    }

    #[traced_test]
    #[tokio::test(flavor = "current_thread")]
    async fn grammar_subset_override_wires_short_grammar_and_logs_source() {
        let mock_server = MockServer::start().await;
        let body = serde_json::json!({
            "choices": [{"message": {"content": "{\"thought\":\"\",\"status\":\"Idle\",\"message_to_user\":null,\"tool_calls\":[]}"}}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1}
        });
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&mock_server)
            .await;

        let config = test_config_with_url(&mock_server.uri(), std::env::temp_dir());
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("http client");
        let chat_url = format!("{}/v1/chat/completions", mock_server.uri());
        let client = LlamaCppClient {
            http,
            chat_url,
            config,
            token_metrics_tx: None,
            grammar: Some(Arc::new("SESSION_GRAMMAR_BLOAT_MARKER".repeat(400))),
        };
        let tiny: Arc<str> = Arc::from("tiny-root-gbnf");
        let stack = vec![Message {
            role: Role::User,
            content: "Hi".into(),
        }];
        client
            .generate(
                &stack,
                "",
                None,
                LlmGenerateOptions {
                    grammar_override: Some(tiny),
                    ..Default::default()
                },
            )
            .await
            .expect("generate");

        let reqs = mock_server.received_requests().await.expect("reqs");
        let posted: serde_json::Value = serde_json::from_slice(&reqs[0].body).expect("body json");
        assert_eq!(
            posted.get("grammar").and_then(|v| v.as_str()),
            Some("tiny-root-gbnf")
        );
        assert!(
            logs_contain("subset_override"),
            "expected grammar_source=subset_override in request log"
        );
        assert!(
            logs_contain("grammar_len"),
            "expected grammar_len field in request log"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn attach_session_grammar_false_omits_grammar_json_field_even_with_session_set() {
        let mock_server = MockServer::start().await;
        let body = serde_json::json!({
            "choices": [{"message": {"content": "x"}}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1}
        });
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&mock_server)
            .await;

        let config = test_config_with_url(&mock_server.uri(), std::env::temp_dir());
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("http client");
        let chat_url = format!("{}/v1/chat/completions", mock_server.uri());
        let client = LlamaCppClient {
            http,
            chat_url,
            config,
            token_metrics_tx: None,
            grammar: Some(Arc::new("large".repeat(500))),
        };
        let stack = vec![Message {
            role: Role::User,
            content: "Hi".into(),
        }];
        client
            .generate(
                &stack,
                "",
                None,
                LlmGenerateOptions {
                    attach_session_grammar: false,
                    ..Default::default()
                },
            )
            .await
            .expect("generate");

        let reqs = mock_server.received_requests().await.expect("reqs");
        let posted: serde_json::Value = serde_json::from_slice(&reqs[0].body).expect("body json");
        assert!(posted.get("grammar").is_none());
    }

    #[tokio::test(flavor = "current_thread")]
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
    fn chat_template_kwargs_when_reasoning_fsm_off() {
        let k = chat_template_kwargs_for_reasoning_config(false).expect("some");
        assert_eq!(k["enable_thinking"], false);
        assert!(chat_template_kwargs_for_reasoning_config(true).is_none());
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

}
