use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};

use crate::config::AppConfig;
use crate::engine::token_metrics::{self, LlmTokenSnapshot};
use crate::engine::{EngineResponse, GenerationConstraints, LlmEngine, Message};
use crate::executive::error::{FcpError, Result};

#[derive(Debug)]
pub struct LlamaServerEngine {
    client: reqwest::Client,
    base_url: String,
    model_name: String,
    timeout: Duration,
    token_metrics_tx: Option<watch::Sender<LlmTokenSnapshot>>,
}

impl LlamaServerEngine {
    pub fn with_token_metrics(
        config: Arc<AppConfig>,
        token_metrics_tx: watch::Sender<LlmTokenSnapshot>,
    ) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .build()
                .map_err(|e| FcpError::Config(format!("Failed to build llama-server HTTP client: {e}")))?,
            base_url: config.llama_server_base_url.trim_end_matches('/').to_string(),
            model_name: config.model_name.clone(),
            timeout: Duration::from_secs(config.generation_timeout_secs),
            token_metrics_tx: Some(token_metrics_tx),
        })
    }

    fn map_messages(stack: &[Message]) -> Vec<Value> {
        stack.iter()
            .map(|msg| {
                let role = match msg.role.as_str() {
                    "system" => "system",
                    "assistant" => "assistant",
                    _ => "user",
                };
                json!({
                    "role": role,
                    "content": msg.content,
                })
            })
            .collect()
    }

    fn trace_request_outline(payload: &Value) {
        let model = payload
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("");
        let msg_count = payload
            .get("messages")
            .and_then(|m| m.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let (rf_type, schema_name, strict, schema_chars) = match payload.get("response_format") {
            Some(rf) => {
                let t = rf.get("type").and_then(|x| x.as_str()).unwrap_or("");
                if t == "json_schema" {
                    let js = rf.get("json_schema");
                    let name = js
                        .and_then(|j| j.get("name"))
                        .and_then(|n| n.as_str())
                        .unwrap_or("");
                    let strict = js
                        .and_then(|j| j.get("strict"))
                        .and_then(|b| b.as_bool())
                        .unwrap_or(false);
                    let chars = js
                        .and_then(|j| j.get("schema"))
                        .and_then(|s| serde_json::to_string(s).ok())
                        .map(|s| s.len())
                        .unwrap_or(0);
                    (t, name, strict, chars)
                } else {
                    (t, "", false, 0)
                }
            }
            None => ("", "", false, 0),
        };
        tracing::info!(
            target: "fcp.llama_server",
            event = "fcp.llama_server.request",
            %model,
            message_count = msg_count,
            response_format = rf_type,
            json_schema_name = schema_name,
            json_schema_strict = strict,
            json_schema_serialized_chars = schema_chars,
            "POST /v1/chat/completions"
        );
    }

    async fn post_chat(&self, payload: Value) -> Result<EngineResponse> {
        let url = format!("{}/v1/chat/completions", self.base_url);
        Self::trace_request_outline(&payload);
        let request = self
            .client
            .post(url)
            .header(CONTENT_TYPE, "application/json")
            .header(AUTHORIZATION, "Bearer llama")
            .json(&payload);

        let response = tokio::time::timeout(self.timeout, request.send())
            .await
            .map_err(|_| FcpError::EngineFault("Generation timed out".to_string()))?
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;
        if !status.is_success() {
            let preview: String = body.chars().take(900).collect();
            tracing::warn!(
                target: "fcp.llama_server",
                event = "fcp.llama_server.http_error",
                status = status.as_u16(),
                body_preview = %preview,
                body_len = body.len(),
                "llama-server rejected request"
            );
            return Err(FcpError::EngineFault(format!(
                "llama-server returned HTTP {}: {}",
                status.as_u16(),
                body
            )));
        }

        let parsed: ChatCompletionResponse = serde_json::from_str(&body)
            .map_err(|e| FcpError::ParseFault(e))?;

        let content = parsed
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .ok_or_else(|| FcpError::EngineFault("Missing assistant content in llama-server response".to_string()))?;

        let prompt_tokens = parsed.usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0);
        let generated_tokens = parsed.usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0);
        tracing::debug!(
            target: "fcp.llama_server",
            event = "fcp.llama_server.response_ok",
            prompt_tokens,
            generated_tokens,
            content_len = content.len(),
            "llama-server chat completion ok"
        );
        token_metrics::publish(&self.token_metrics_tx, prompt_tokens, generated_tokens);

        Ok(EngineResponse {
            content,
            prompt_tokens,
            generated_tokens,
        })
    }

    pub async fn probe_strict_json_schema_support(&self) -> Result<()> {
        let constraints = GenerationConstraints::new(
            json!({
                "type": "object",
                "properties": {
                    "ok": {"type": "boolean"}
                },
                "required": ["ok"],
                "additionalProperties": false
            }),
            "eris_probe_schema",
        );
        let probe_stack = vec![Message {
            role: "system".to_string(),
            content: "Return a JSON object that satisfies the schema.".to_string(),
        }];
        let response = self
            .generate_constrained(&probe_stack, "", &constraints, None)
            .await?;
        let parsed: Value =
            serde_json::from_str(&response.content).map_err(FcpError::ParseFault)?;
        let ok = parsed
            .get("ok")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| {
                FcpError::EngineFault(
                    "llama-server strict schema probe returned invalid payload".to_string(),
                )
            })?;
        if !ok {
            return Err(FcpError::EngineFault(
                "llama-server strict schema probe returned `ok=false`".to_string(),
            ));
        }
        tracing::info!(
            target: "fcp.llama_server",
            event = "fcp.llama_server.probe_ok",
            "Strict json_schema probe succeeded against llama-server"
        );
        Ok(())
    }
}

#[async_trait]
impl LlmEngine for LlamaServerEngine {
    async fn generate(
        &self,
        stack: &[Message],
        _available_tools_json: &str,
        stream_tx: Option<mpsc::UnboundedSender<String>>,
    ) -> Result<EngineResponse> {
        if stream_tx.is_some() {
            return Err(FcpError::EngineFault(
                "Streaming is not implemented for llama-server backend".to_string(),
            ));
        }
        let payload = json!({
            "model": self.model_name,
            "messages": Self::map_messages(stack),
            "response_format": {
                "type": "json_object"
            }
        });
        self.post_chat(payload).await
    }

    async fn generate_constrained(
        &self,
        stack: &[Message],
        _available_tools_json: &str,
        constraints: &GenerationConstraints,
        stream_tx: Option<mpsc::UnboundedSender<String>>,
    ) -> Result<EngineResponse> {
        if stream_tx.is_some() {
            return Err(FcpError::EngineFault(
                "Streaming is not implemented for llama-server backend".to_string(),
            ));
        }
        let payload = json!({
            "model": self.model_name,
            "messages": Self::map_messages(stack),
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": constraints.schema_name,
                    "strict": constraints.strict,
                    "schema": constraints.schema,
                }
            }
        });
        self.post_chat(payload).await
    }
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct ChatChoiceMessage {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Usage {
    #[serde(default)]
    prompt_tokens: usize,
    #[serde(default)]
    completion_tokens: usize,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::config::AppConfig;
    use crate::engine::{GenerationConstraints, LlmEngine, Message};

    use super::LlamaServerEngine;

    #[tokio::test]
    async fn constrained_payload_uses_strict_json_schema_wrapper() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(serde_json::json!({
                "response_format": {
                    "type": "json_schema",
                    "json_schema": {
                        "name": "eris_protocol_response",
                        "strict": true
                    }
                }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "{\"ok\":true}"}}],
                "usage": {"prompt_tokens": 4, "completion_tokens": 2}
            })))
            .mount(&server)
            .await;

        let mut cfg = AppConfig::default();
        cfg.llama_server_base_url = server.uri();
        cfg.generation_timeout_secs = 5;
        let engine = LlamaServerEngine::with_token_metrics(
            Arc::new(cfg),
            crate::engine::token_metrics::channel().0,
        )
        .expect("engine");

        let constraints = GenerationConstraints::new(
            serde_json::json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"]}),
            "eris_protocol_response",
        );
        let out = engine
            .generate_constrained(
                &[Message {
                    role: "system".to_string(),
                    content: "test".to_string(),
                }],
                "",
                &constraints,
                None,
            )
            .await
            .expect("response");

        assert_eq!(out.prompt_tokens, 4);
        assert_eq!(out.generated_tokens, 2);
    }

    #[tokio::test]
    async fn generation_timeout_maps_to_engine_fault() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
            .mount(&server)
            .await;

        let mut cfg = AppConfig::default();
        cfg.llama_server_base_url = server.uri();
        cfg.generation_timeout_secs = 1;
        let engine = LlamaServerEngine::with_token_metrics(
            Arc::new(cfg),
            crate::engine::token_metrics::channel().0,
        )
        .expect("engine");

        let err = engine
            .generate(
                &[Message {
                    role: "system".to_string(),
                    content: "test".to_string(),
                }],
                "",
                None,
            )
            .await
            .expect_err("timeout");
        assert!(matches!(err, crate::executive::error::FcpError::EngineFault(_)));
    }
}
