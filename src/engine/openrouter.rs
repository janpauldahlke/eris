//! OpenRouter chat backend: the OpenAI-compatible `/chat/completions` API over HTTPS with a
//! Bearer key from the environment. Adapted from [`crate::engine::llama_cpp::LlamaCppClient`].
//!
//! Envelope enforcement prefers native `tools` / `tool_choice` (projected back into the FCP
//! envelope at the orchestrator). The strict `response_format` JSON Schema path remains the
//! session downgrade when a model rejects `tools` (HTTP 400).
//!
//! Requests always stream (SSE): dropping the generate future aborts the HTTP request, which is
//! the orchestrator's interrupt mechanism — a non-streaming hosted call would keep running (and
//! billing) server-side until completion. Native tool-call arguments are assembled from
//! `delta.tool_calls[]` fragments (Phase 4 Solution B) so cost and interrupt stay uniform.

use crate::config::{
    AppConfig, DataCollection, OpenRouterConfig, OpenRouterReasoning, ResponseFormatMode,
};
use crate::engine::openai_wire::{ChatMsg, to_wire_messages};
use crate::engine::structured::OpenAiNativeTool;
use crate::engine::token_metrics::{self, LlmTokenSnapshot};
use crate::engine::{
    EngineResponse, EngineToolCall, LlmEngine, LlmGenerateOptions, Message, ToolChoice,
};
use crate::executive::error::{FcpError, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};

/// Session-level structured-output ladder position (only ever downgraded, never upgraded).
const MODE_JSON_SCHEMA: u8 = 0;
const MODE_JSON_OBJECT: u8 = 1;
const MODE_OFF: u8 = 2;

pub struct OpenRouterClient {
    http: reqwest::Client,
    chat_url: String,
    or_config: OpenRouterConfig,
    #[allow(dead_code)]
    config: Arc<AppConfig>,
    token_metrics_tx: Option<watch::Sender<LlmTokenSnapshot>>,
    /// Effective [`ResponseFormatMode`] for this session; starts at the configured mode and is
    /// downgraded (json_schema → json_object → off) when a model rejects the richer form with
    /// HTTP 400. Atomic because `generate` takes `&self` (no shared `Mutex` per `.cursorrules`).
    effective_format_mode: AtomicU8,
    /// When false, this session no longer sends `tools[]` and uses envelope `response_format`
    /// instead (model rejected native tools with HTTP 400).
    native_tools_enabled: AtomicBool,
}

fn mode_to_u8(mode: ResponseFormatMode) -> u8 {
    match mode {
        ResponseFormatMode::JsonSchema => MODE_JSON_SCHEMA,
        ResponseFormatMode::JsonObject => MODE_JSON_OBJECT,
        ResponseFormatMode::Off => MODE_OFF,
    }
}

impl OpenRouterClient {
    /// Build the client: validates config, enforces the consent gate, and reads the API key
    /// from the environment exactly once. The key lives only in the HTTP client's default
    /// headers and is never logged or serialized.
    pub fn new(config: Arc<AppConfig>) -> Result<Self> {
        let or = config.validate_openrouter_config()?.clone();
        if !or.consent_acknowledged {
            return Err(FcpError::Config(
                "OpenRouter consent gate: set openrouter.consent_acknowledged = true to confirm \
                 that chat content (vault excerpts, tool outputs, memories, condensation summaries) \
                 may be sent to OpenRouter and its providers. No request is sent without it."
                    .into(),
            ));
        }
        let api_key = std::env::var(&or.api_key_env).map_err(|_| {
            FcpError::Config(format!(
                "OpenRouter API key env var `{}` is not set",
                or.api_key_env
            ))
        })?;

        let mut headers = reqwest::header::HeaderMap::new();
        let mut auth =
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", api_key.trim())).map_err(
                |_| {
                    FcpError::Config("OpenRouter API key contains invalid header characters".into())
                },
            )?;
        auth.set_sensitive(true);
        headers.insert(reqwest::header::AUTHORIZATION, auth);
        if let Some(ref referer) = or.referer
            && let Ok(v) = reqwest::header::HeaderValue::from_str(referer)
        {
            headers.insert("HTTP-Referer", v);
        }
        if let Some(ref title) = or.title
            && let Ok(v) = reqwest::header::HeaderValue::from_str(title)
        {
            headers.insert("X-Title", v);
        }

        let http = reqwest::Client::builder()
            .default_headers(headers)
            .user_agent(concat!("eris/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(or.request_timeout_secs))
            .read_timeout(Duration::from_secs(or.stream_idle_timeout_secs))
            .build()
            .map_err(|e| FcpError::NetworkFault(format!("HTTP client build: {e}")))?;

        let chat_url = format!("{}/chat/completions", or.base_url.trim_end_matches('/'));
        let effective_format_mode = AtomicU8::new(mode_to_u8(or.response_format_mode));

        Ok(Self {
            http,
            chat_url,
            or_config: or,
            config,
            token_metrics_tx: None,
            effective_format_mode,
            native_tools_enabled: AtomicBool::new(true),
        })
    }

    pub fn with_token_metrics(mut self, tx: watch::Sender<LlmTokenSnapshot>) -> Self {
        self.token_metrics_tx = Some(tx);
        self
    }

    /// `response_format` value for this request, honoring the session downgrade ladder.
    /// Internal prose passes (condensation: `attach_session_grammar == false` and no explicit
    /// schema) get no `response_format` at all — they are not FCP envelope JSON.
    fn response_format_for(
        &self,
        options: &LlmGenerateOptions,
        mode: u8,
    ) -> Option<serde_json::Value> {
        if !options.attach_session_grammar && options.response_json_schema.is_none() {
            return None;
        }
        match mode {
            MODE_JSON_SCHEMA => match options.response_json_schema.as_deref() {
                Some(schema) => Some(serde_json::json!({
                    "type": "json_schema",
                    "json_schema": {
                        "name": "fcp_envelope",
                        "strict": true,
                        "schema": schema,
                    }
                })),
                // No per-hop schema available: still guarantee valid JSON.
                None => Some(serde_json::json!({ "type": "json_object" })),
            },
            MODE_JSON_OBJECT => Some(serde_json::json!({ "type": "json_object" })),
            _ => None,
        }
    }

    /// Downgrade the session ladder one step; returns `true` when a downgrade happened.
    fn downgrade_format_mode(&self, from: u8) -> bool {
        if from >= MODE_OFF {
            return false;
        }
        let to = from + 1;
        // Only downgrade if nobody else already moved past `from` (races are harmless).
        let _ = self.effective_format_mode.compare_exchange(
            from,
            to,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
        tracing::warn!(
            from_mode = from,
            to_mode = to,
            "OpenRouter rejected structured output; downgrading response_format for this session"
        );
        true
    }
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Serialize)]
struct UsageInclude {
    include: bool,
}

#[derive(Serialize)]
struct ProviderPrefs {
    #[serde(skip_serializing_if = "Option::is_none")]
    require_parameters: Option<bool>,
    data_collection: &'static str,
}

#[derive(Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMsg],
    stream: bool,
    stream_options: StreamOptions,
    /// OpenRouter accounting: include `usage.cost` (credits) in the final chunk.
    usage: UsageInclude,
    provider: ProviderPrefs,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [OpenAiNativeTool]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<ToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<serde_json::Value>,
    /// Ordered fallback list (primary first) with `route: "fallback"` for provider outages.
    #[serde(skip_serializing_if = "Option::is_none")]
    models: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    route: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<&'a [String]>,
}

fn reasoning_request_value(reasoning: &OpenRouterReasoning) -> Option<serde_json::Value> {
    match reasoning {
        OpenRouterReasoning::Off => None,
        OpenRouterReasoning::Effort(effort) => Some(serde_json::json!({ "effort": effort })),
        OpenRouterReasoning::MaxTokens(n) => Some(serde_json::json!({ "max_tokens": n })),
    }
}

#[derive(Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<Usage>,
    #[serde(default)]
    error: Option<ApiError>,
}

#[derive(Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: Option<Delta>,
}

#[derive(Deserialize)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
    /// Hosted reasoning arrives on a separate field and never contaminates the envelope.
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    tool_calls: Vec<DeltaToolCall>,
}

#[derive(Deserialize, Default)]
struct DeltaToolCall {
    #[serde(default)]
    index: Option<u32>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<DeltaFunction>,
}

#[derive(Deserialize, Default)]
struct DeltaFunction {
    #[serde(default)]
    name: Option<String>,
    /// OpenAI sends a JSON **string** fragment; some gateways send a full object once.
    #[serde(default)]
    arguments: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    prompt_tokens: Option<usize>,
    #[serde(default)]
    completion_tokens: Option<usize>,
    /// OpenRouter credits (USD) when `usage: {include: true}` was requested.
    #[serde(default)]
    cost: Option<f64>,
    #[serde(default)]
    completion_tokens_details: Option<CompletionTokensDetails>,
}

#[derive(Deserialize)]
struct CompletionTokensDetails {
    #[serde(default)]
    reasoning_tokens: Option<usize>,
}

#[derive(Deserialize)]
struct ApiError {
    #[serde(default)]
    message: Option<String>,
}

#[derive(Default)]
struct StreamOutcome {
    content: String,
    reasoning: String,
    tool_calls: Vec<EngineToolCall>,
    prompt_tokens: usize,
    completion_tokens: usize,
    reasoning_tokens: usize,
    reported_cost_usd: Option<f64>,
}

#[derive(Default)]
struct PartialToolCall {
    id: Option<String>,
    name: String,
    arguments: String,
}

fn apply_tool_call_delta(acc: &mut BTreeMap<u32, PartialToolCall>, delta: &DeltaToolCall) {
    let idx = delta.index.unwrap_or(0);
    let entry = acc.entry(idx).or_default();
    if let Some(id) = delta.id.as_deref().filter(|s| !s.is_empty()) {
        entry.id = Some(id.to_string());
    }
    let Some(function) = delta.function.as_ref() else {
        return;
    };
    if let Some(name) = function.name.as_deref().filter(|s| !s.is_empty()) {
        entry.name.push_str(name);
    }
    if let Some(args) = &function.arguments {
        match args {
            serde_json::Value::String(s) => entry.arguments.push_str(s),
            other => {
                // Full object in one chunk (non-fragmented gateway).
                if entry.arguments.is_empty() {
                    entry.arguments = other.to_string();
                }
            }
        }
    }
}

fn finalize_tool_calls(acc: BTreeMap<u32, PartialToolCall>) -> Vec<EngineToolCall> {
    acc.into_values()
        .filter(|p| !p.name.is_empty())
        .map(|p| EngineToolCall {
            id: p.id,
            name: p.name,
            arguments: p.arguments,
        })
        .collect()
}

/// Parse the OpenRouter SSE stream: `data:` frames, `[DONE]` sentinel, comment keep-alives
/// (`: OPENROUTER PROCESSING`), and mid-stream error objects (a `data:` chunk carrying an
/// `error` field must surface as a fault, not be skipped).
async fn consume_sse_stream(
    response: reqwest::Response,
    stream_tx: Option<&mpsc::UnboundedSender<String>>,
) -> Result<StreamOutcome> {
    use futures::StreamExt;

    let mut out = StreamOutcome::default();
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut tool_acc: BTreeMap<u32, PartialToolCall> = BTreeMap::new();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result
            .map_err(|e| FcpError::NetworkFault(format!("OpenRouter stream read: {e}")))?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].trim().to_string();
            buffer = buffer[newline_pos + 1..].to_string();

            if line.is_empty() || line.starts_with(':') {
                // SSE comment keep-alive (": OPENROUTER PROCESSING") or frame separator.
                continue;
            }
            let Some(data) = line.strip_prefix("data: ").map(str::trim) else {
                continue;
            };
            if data == "[DONE]" {
                out.tool_calls = finalize_tool_calls(tool_acc);
                return Ok(out);
            }
            let parsed: StreamChunk = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if let Some(err) = parsed.error {
                return Err(FcpError::EngineFault(format!(
                    "OpenRouter mid-stream error: {}",
                    err.message.unwrap_or_else(|| "unknown".into())
                )));
            }
            if let Some(usage) = &parsed.usage {
                out.prompt_tokens = usage.prompt_tokens.unwrap_or(out.prompt_tokens);
                out.completion_tokens = usage.completion_tokens.unwrap_or(out.completion_tokens);
                if let Some(cost) = usage.cost {
                    out.reported_cost_usd = Some(cost);
                }
                if let Some(details) = &usage.completion_tokens_details {
                    out.reasoning_tokens = details.reasoning_tokens.unwrap_or(out.reasoning_tokens);
                }
            }
            if let Some(delta) = parsed.choices.first().and_then(|c| c.delta.as_ref()) {
                if let Some(reasoning) = &delta.reasoning {
                    out.reasoning.push_str(reasoning);
                }
                if let Some(content) = &delta.content {
                    out.content.push_str(content);
                    if let Some(tx) = stream_tx {
                        let _ = tx.send(content.clone());
                    }
                }
                for tc in &delta.tool_calls {
                    apply_tool_call_delta(&mut tool_acc, tc);
                }
            }
        }
    }

    out.tool_calls = finalize_tool_calls(tool_acc);
    Ok(out)
}

/// USD → micro-USD, saturating; `None` when no cost source is available (local pricing unset
/// and no reported credits).
fn cost_micro_usd(
    reported_cost_usd: Option<f64>,
    prompt_tokens: usize,
    completion_tokens: usize,
    or: &OpenRouterConfig,
) -> Option<u64> {
    let usd =
        reported_cost_usd.or_else(|| match (or.price_per_mtok_in, or.price_per_mtok_out) {
            (None, None) => None,
            (input, output) => Some(
                (prompt_tokens as f64) * input.unwrap_or(0.0) / 1_000_000.0
                    + (completion_tokens as f64) * output.unwrap_or(0.0) / 1_000_000.0,
            ),
        })?;
    if !usd.is_finite() || usd <= 0.0 {
        return Some(0);
    }
    Some((usd * 1_000_000.0).round() as u64)
}

/// Retry-eligible transient statuses (429 + upstream 5xx).
fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn retry_after_secs(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
}

#[async_trait]
impl LlmEngine for OpenRouterClient {
    async fn generate(
        &self,
        stack: &[Message],
        _available_tools_json: &str,
        stream_tx: Option<mpsc::UnboundedSender<String>>,
        options: LlmGenerateOptions,
    ) -> Result<EngineResponse> {
        let messages = to_wire_messages(stack);
        let message_count = messages.len();
        let or = &self.or_config;
        let temperature = options.temperature.unwrap_or(0.7f32);
        let max_tokens = (or.max_tokens > 0).then_some(or.max_tokens);
        let reasoning = reasoning_request_value(&or.reasoning);
        let (models, route) = if or.fallback_models.is_empty() {
            (None, None)
        } else {
            let mut list = Vec::with_capacity(1 + or.fallback_models.len());
            list.push(or.model.clone());
            list.extend(or.fallback_models.iter().cloned());
            (Some(list), Some("fallback"))
        };

        let gen_started = Instant::now();
        let mut attempt: u32 = 0;

        loop {
            let mode = self.effective_format_mode.load(Ordering::SeqCst);
            let tools_slice: Option<&[OpenAiNativeTool]> =
                if self.native_tools_enabled.load(Ordering::SeqCst) {
                    options
                        .native_tools
                        .as_deref()
                        .filter(|t| !t.is_empty())
                        .map(|t| t.as_slice())
                } else {
                    None
                };
            let tools_attached = tools_slice.is_some();
            // Native tools and strict envelope `response_format` are mutually exclusive.
            let response_format = if tools_attached {
                None
            } else {
                self.response_format_for(&options, mode)
            };
            let structured_attached = response_format.is_some();
            let tool_choice = if tools_attached {
                options.tool_choice
            } else {
                None
            };
            let request_body = ChatCompletionRequest {
                model: &or.model,
                messages: &messages,
                stream: true,
                stream_options: StreamOptions {
                    include_usage: true,
                },
                usage: UsageInclude { include: true },
                provider: ProviderPrefs {
                    // Pin providers that honor tools / response_format; otherwise a router hop
                    // can silently drop the constraint and return unconstrained prose at HTTP 200.
                    require_parameters: ((tools_attached || structured_attached)
                        && or.require_parameters)
                        .then_some(true),
                    data_collection: match or.data_collection {
                        DataCollection::Deny => "deny",
                        DataCollection::Allow => "allow",
                    },
                },
                temperature: Some(temperature),
                max_tokens,
                response_format,
                tools: tools_slice,
                tool_choice,
                reasoning: reasoning.clone(),
                models: models.clone(),
                route,
                seed: or.seed,
                top_p: or.top_p,
                stop: (!or.stop.is_empty()).then_some(or.stop.as_slice()),
            };

            tracing::info!(
                engine = "openrouter",
                model = %or.model,
                message_count,
                attempt,
                structured_mode = mode,
                structured_attached,
                tools_attached,
                tool_choice = ?tool_choice,
                reasoning_requested = reasoning.is_some(),
                "Sending chat request to OpenRouter"
            );

            let send_result = self
                .http
                .post(&self.chat_url)
                .json(&request_body)
                .send()
                .await;
            let response = match send_result {
                Ok(r) => r,
                Err(e) => {
                    let transient = e.is_connect();
                    if transient && attempt < or.max_retries {
                        attempt += 1;
                        let backoff =
                            Duration::from_millis(500u64.saturating_mul(1 << attempt.min(6)));
                        tracing::warn!(error = %e, attempt, "OpenRouter connect failed; retrying");
                        tokio::time::sleep(backoff).await;
                        continue;
                    }
                    return Err(if e.is_timeout() {
                        FcpError::NetworkFault("OpenRouter request timed out".into())
                    } else if e.is_connect() {
                        FcpError::NetworkFault(format!(
                            "OpenRouter unreachable at {} — check network / base_url",
                            self.chat_url
                        ))
                    } else {
                        FcpError::NetworkFault(format!("OpenRouter request failed: {e}"))
                    });
                }
            };

            let status = response.status();
            if !status.is_success() {
                let retry_after = retry_after_secs(&response);
                let body_excerpt = response
                    .text()
                    .await
                    .unwrap_or_default()
                    .chars()
                    .take(500)
                    .collect::<String>();

                if status == reqwest::StatusCode::UNAUTHORIZED {
                    return Err(FcpError::Config(
                        "OpenRouter rejected the API key (HTTP 401). Check the key env var.".into(),
                    ));
                }
                if status == reqwest::StatusCode::PAYMENT_REQUIRED {
                    return Err(FcpError::NetworkFault(format!(
                        "OpenRouter: insufficient credits (HTTP 402): {body_excerpt}"
                    )));
                }
                if status == reqwest::StatusCode::BAD_REQUEST {
                    // Native tools unsupported: drop `tools[]` for this session and retry with
                    // the envelope `response_format` path.
                    if tools_attached
                        && self
                            .native_tools_enabled
                            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
                            .is_ok()
                    {
                        tracing::warn!(
                            http_status = %status,
                            body = %body_excerpt,
                            "OpenRouter HTTP 400 with tools attached; downgrading session to response_format envelope"
                        );
                        continue;
                    }
                    // Model rejected the richer request shape (strict json_schema, or
                    // reasoning + strict schema): downgrade the ladder and retry in-place.
                    if structured_attached && self.downgrade_format_mode(mode) {
                        tracing::warn!(
                            http_status = %status,
                            body = %body_excerpt,
                            "OpenRouter HTTP 400 with structured output attached; retrying downgraded"
                        );
                        continue;
                    }
                    return Err(FcpError::EngineFault(format!(
                        "OpenRouter rejected the request (HTTP 400): {body_excerpt}"
                    )));
                }
                if is_retryable_status(status) {
                    if attempt < or.max_retries {
                        attempt += 1;
                        let backoff = retry_after.map(Duration::from_secs).unwrap_or_else(|| {
                            Duration::from_millis(500u64.saturating_mul(1 << attempt.min(6)))
                        });
                        tracing::warn!(
                            http_status = %status,
                            attempt,
                            backoff_ms = backoff.as_millis() as u64,
                            "OpenRouter transient error; retrying"
                        );
                        tokio::time::sleep(backoff).await;
                        continue;
                    }
                    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        return Err(FcpError::RateLimited {
                            retry_after_secs: retry_after,
                        });
                    }
                }
                return Err(FcpError::NetworkFault(format!(
                    "OpenRouter returned HTTP {status}: {body_excerpt}"
                )));
            }

            let outcome = consume_sse_stream(response, stream_tx.as_ref()).await?;

            let generation_ms = gen_started.elapsed().as_millis() as u64;
            let cost = cost_micro_usd(
                outcome.reported_cost_usd,
                outcome.prompt_tokens,
                outcome.completion_tokens,
                or,
            );
            token_metrics::publish_with_cost(
                &self.token_metrics_tx,
                outcome.prompt_tokens,
                outcome.completion_tokens,
                generation_ms,
                outcome.reasoning_tokens,
                cost,
            );

            if !outcome.reasoning.is_empty() {
                // Native reasoning is optional telemetry — never re-injected into the paid
                // chat stack; the FCP envelope `thought` stays the source of truth.
                tracing::debug!(
                    target: "fcp.model_thought",
                    engine = "openrouter",
                    reasoning_len = outcome.reasoning.len(),
                    reasoning = %outcome.reasoning,
                    "Hosted reasoning trace"
                );
            }

            tracing::info!(
                engine = "openrouter",
                model = %or.model,
                prompt_tokens = outcome.prompt_tokens,
                completion_tokens = outcome.completion_tokens,
                reasoning_tokens = outcome.reasoning_tokens,
                cost_micro_usd = cost,
                generation_ms,
                content_len = outcome.content.len(),
                native_tool_calls = outcome.tool_calls.len(),
                "OpenRouter chat response complete"
            );

            return Ok(EngineResponse {
                content: outcome.content,
                tool_calls: outcome.tool_calls,
                reasoning: outcome.reasoning,
                prompt_tokens: outcome.prompt_tokens,
                generated_tokens: outcome.completion_tokens,
                generation_ms,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ReasoningEffort;
    use crate::engine::ToolChoice;
    use crate::engine::structured::OpenAiNativeTool;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    fn test_or_config() -> OpenRouterConfig {
        OpenRouterConfig {
            model: "test/model-1".into(),
            consent_acknowledged: true,
            max_retries: 1,
            request_timeout_secs: 5,
            stream_idle_timeout_secs: 5,
            ..Default::default()
        }
    }

    /// Client wired directly at a mock server (no env access; constructor env reads are
    /// covered by config validation tests).
    fn make_client(mock_url: &str, or: OpenRouterConfig) -> OpenRouterClient {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_static("Bearer test-key-do-not-log"),
        );
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(5))
            .build()
            .expect("http client");
        let effective_format_mode = AtomicU8::new(mode_to_u8(or.response_format_mode));
        OpenRouterClient {
            http,
            chat_url: format!("{}/chat/completions", mock_url),
            or_config: or,
            config: Arc::new(AppConfig::default()),
            token_metrics_tx: None,
            effective_format_mode,
            native_tools_enabled: AtomicBool::new(true),
        }
    }

    fn sse_body_ok() -> String {
        ": OPENROUTER PROCESSING\n\n\
         data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n\
         data: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\n\
         data: {\"choices\":[{\"delta\":{}}],\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":4,\"cost\":0.00123}}\n\n\
         data: [DONE]\n\n"
            .to_string()
    }

    fn user_stack() -> Vec<Message> {
        vec![Message::user("Hi")]
    }

    async fn posted_body(mock_server: &MockServer) -> serde_json::Value {
        let reqs = mock_server.received_requests().await.expect("reqs");
        serde_json::from_slice(&reqs[0].body).expect("body json")
    }

    #[tokio::test(flavor = "current_thread")]
    async fn streaming_response_with_usage_and_auth_header() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("authorization", "Bearer test-key-do-not-log"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse_body_ok()))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = make_client(&mock_server.uri(), test_or_config());
        let result = client
            .generate(&user_stack(), "", None, LlmGenerateOptions::default())
            .await
            .expect("generate");
        assert_eq!(result.content, "Hello world");
        assert_eq!(result.prompt_tokens, 11);
        assert_eq!(result.generated_tokens, 4);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn request_is_streaming_with_usage_and_provider_prefs() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse_body_ok()))
            .mount(&mock_server)
            .await;

        let client = make_client(&mock_server.uri(), test_or_config());
        client
            .generate(&user_stack(), "", None, LlmGenerateOptions::default())
            .await
            .expect("generate");

        let body = posted_body(&mock_server).await;
        assert_eq!(body["model"], "test/model-1");
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["usage"]["include"], true);
        assert_eq!(body["provider"]["data_collection"], "deny");
        assert_eq!(body["max_tokens"], 2048);
        assert!(body.get("grammar").is_none(), "GBNF is llama-server-only");
        assert!(
            body.get("n_predict").is_none(),
            "n_predict is llama-server-only"
        );
        assert!(
            body.get("chat_template_kwargs").is_none(),
            "chat_template_kwargs is llama-server-only"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn json_schema_mode_sends_strict_schema_and_pins_require_parameters() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse_body_ok()))
            .mount(&mock_server)
            .await;

        let client = make_client(&mock_server.uri(), test_or_config());
        let schema = Arc::new(serde_json::json!({"type": "object"}));
        client
            .generate(
                &user_stack(),
                "",
                None,
                LlmGenerateOptions {
                    response_json_schema: Some(schema),
                    ..Default::default()
                },
            )
            .await
            .expect("generate");

        let body = posted_body(&mock_server).await;
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(
            body["response_format"]["json_schema"]["name"],
            "fcp_envelope"
        );
        assert_eq!(body["response_format"]["json_schema"]["strict"], true);
        assert_eq!(
            body["response_format"]["json_schema"]["schema"]["type"],
            "object"
        );
        assert_eq!(body["provider"]["require_parameters"], true);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn response_format_omitted_when_mode_off_and_for_internal_passes() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse_body_ok()))
            .mount(&mock_server)
            .await;

        let mut or = test_or_config();
        or.response_format_mode = ResponseFormatMode::Off;
        let client = make_client(&mock_server.uri(), or);
        client
            .generate(&user_stack(), "", None, LlmGenerateOptions::default())
            .await
            .expect("generate");

        let body = posted_body(&mock_server).await;
        assert!(body.get("response_format").is_none());
        assert!(body["provider"].get("require_parameters").is_none());

        // Internal summarization pass (condensation): no response_format even in JsonSchema mode.
        let client2 = make_client(&mock_server.uri(), test_or_config());
        client2
            .generate(
                &user_stack(),
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
        let body2: serde_json::Value = serde_json::from_slice(&reqs[1].body).expect("json");
        assert!(body2.get("response_format").is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn http_400_downgrades_json_schema_to_json_object_and_succeeds() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(move |req: &Request| {
                let body: serde_json::Value =
                    serde_json::from_slice(&req.body).expect("request json");
                let is_schema = body
                    .get("response_format")
                    .and_then(|rf| rf.get("type"))
                    .and_then(|t| t.as_str())
                    == Some("json_schema");
                if is_schema {
                    ResponseTemplate::new(400).set_body_string(
                        r#"{"error":{"message":"response_format json_schema unsupported"}}"#,
                    )
                } else {
                    ResponseTemplate::new(200).set_body_string(sse_body_ok())
                }
            })
            // 3 requests: json_schema (400) + in-place json_object retry, then the
            // second generate call proving the session-level downgrade persists.
            .expect(3)
            .mount(&mock_server)
            .await;

        let client = make_client(&mock_server.uri(), test_or_config());
        let schema = Arc::new(serde_json::json!({"type": "object"}));
        let result = client
            .generate(
                &user_stack(),
                "",
                None,
                LlmGenerateOptions {
                    response_json_schema: Some(schema.clone()),
                    ..Default::default()
                },
            )
            .await
            .expect("generate after downgrade");
        assert_eq!(result.content, "Hello world");
        assert_eq!(
            client.effective_format_mode.load(Ordering::SeqCst),
            MODE_JSON_OBJECT,
            "session downgrade persists"
        );

        // Next call goes straight to json_object.
        client
            .generate(
                &user_stack(),
                "",
                None,
                LlmGenerateOptions {
                    response_json_schema: Some(schema),
                    ..Default::default()
                },
            )
            .await
            .expect("generate at downgraded mode");
        let reqs = mock_server.received_requests().await.expect("reqs");
        let last: serde_json::Value =
            serde_json::from_slice(&reqs.last().expect("last").body).expect("json");
        assert_eq!(last["response_format"]["type"], "json_object");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn http_401_maps_to_config_fault() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .mount(&mock_server)
            .await;

        let client = make_client(&mock_server.uri(), test_or_config());
        let err = client
            .generate(&user_stack(), "", None, LlmGenerateOptions::default())
            .await
            .unwrap_err();
        assert!(matches!(err, FcpError::Config(_)), "{err}");
        assert!(err.to_string().contains("401"), "{err}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn http_429_exhausts_retries_then_rate_limited() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "1")
                    .set_body_string("slow down"),
            )
            .expect(2) // initial + max_retries(1)
            .mount(&mock_server)
            .await;

        let client = make_client(&mock_server.uri(), test_or_config());
        let err = client
            .generate(&user_stack(), "", None, LlmGenerateOptions::default())
            .await
            .unwrap_err();
        match err {
            FcpError::RateLimited { retry_after_secs } => {
                assert_eq!(retry_after_secs, Some(1));
            }
            other => panic!("expected RateLimited, got {other}"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn http_500_retries_then_network_fault() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("upstream exploded"))
            .expect(2)
            .mount(&mock_server)
            .await;

        let client = make_client(&mock_server.uri(), test_or_config());
        let err = client
            .generate(&user_stack(), "", None, LlmGenerateOptions::default())
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("500"), "{msg}");
        assert!(msg.contains("upstream exploded"), "{msg}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mid_stream_error_object_surfaces_as_fault() {
        let mock_server = MockServer::start().await;
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n\
                   data: {\"error\":{\"message\":\"provider fell over\"}}\n\n\
                   data: [DONE]\n\n";
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse))
            .mount(&mock_server)
            .await;

        let client = make_client(&mock_server.uri(), test_or_config());
        let err = client
            .generate(&user_stack(), "", None, LlmGenerateOptions::default())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("provider fell over"), "{err}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn deltas_forwarded_and_reasoning_kept_out_of_content() {
        let mock_server = MockServer::start().await;
        let sse = "data: {\"choices\":[{\"delta\":{\"reasoning\":\"let me think\"}}]}\n\n\
                   data: {\"choices\":[{\"delta\":{\"content\":\"{\\\"thought\\\":\\\"x\\\"}\"}}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":9,\"completion_tokens_details\":{\"reasoning_tokens\":7}}}\n\n\
                   data: [DONE]\n\n";
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse))
            .mount(&mock_server)
            .await;

        let (metrics_tx, metrics_rx) = token_metrics::channel();
        let client =
            make_client(&mock_server.uri(), test_or_config()).with_token_metrics(metrics_tx);
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let result = client
            .generate(&user_stack(), "", Some(tx), LlmGenerateOptions::default())
            .await
            .expect("generate");

        assert_eq!(result.content, "{\"thought\":\"x\"}");
        assert!(!result.content.contains("let me think"));
        let mut deltas = Vec::new();
        while let Ok(d) = rx.try_recv() {
            deltas.push(d);
        }
        assert_eq!(deltas, vec!["{\"thought\":\"x\"}"]);

        let snap = token_metrics::TokenMetricsReader::new(metrics_rx).snapshot();
        assert_eq!(snap.prompt_tokens, 5);
        assert_eq!(snap.generated_tokens, 9);
        assert_eq!(snap.last_reasoning_tokens, 7);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cost_from_reported_credits_and_from_local_pricing() {
        // Reported credits win.
        let or = test_or_config();
        assert_eq!(cost_micro_usd(Some(0.00123), 11, 4, &or), Some(1230));

        // Local pricing fallback: 1M in @ $0.15 + 1M out @ $0.60.
        let mut priced = test_or_config();
        priced.price_per_mtok_in = Some(0.15);
        priced.price_per_mtok_out = Some(0.60);
        assert_eq!(
            cost_micro_usd(None, 1_000_000, 1_000_000, &priced),
            Some(750_000)
        );

        // No source at all: unknown, not zero.
        assert_eq!(cost_micro_usd(None, 11, 4, &or), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn token_metrics_carry_cost_from_stream() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse_body_ok()))
            .mount(&mock_server)
            .await;

        let (metrics_tx, metrics_rx) = token_metrics::channel();
        let client =
            make_client(&mock_server.uri(), test_or_config()).with_token_metrics(metrics_tx);
        client
            .generate(&user_stack(), "", None, LlmGenerateOptions::default())
            .await
            .expect("generate");
        let snap = token_metrics::TokenMetricsReader::new(metrics_rx).snapshot();
        assert_eq!(snap.last_cost_micro_usd, Some(1230));
        assert_eq!(snap.session_cost_micro_usd, 1230);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reasoning_config_serialized_per_variant() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse_body_ok()))
            .mount(&mock_server)
            .await;

        let mut or = test_or_config();
        or.reasoning = OpenRouterReasoning::Effort(ReasoningEffort::High);
        let client = make_client(&mock_server.uri(), or);
        client
            .generate(&user_stack(), "", None, LlmGenerateOptions::default())
            .await
            .expect("generate");
        let body = posted_body(&mock_server).await;
        assert_eq!(body["reasoning"]["effort"], "high");

        let mut or2 = test_or_config();
        or2.reasoning = OpenRouterReasoning::MaxTokens(512);
        let client2 = make_client(&mock_server.uri(), or2);
        client2
            .generate(&user_stack(), "", None, LlmGenerateOptions::default())
            .await
            .expect("generate");
        let reqs = mock_server.received_requests().await.expect("reqs");
        let body2: serde_json::Value = serde_json::from_slice(&reqs[1].body).expect("json");
        assert_eq!(body2["reasoning"]["max_tokens"], 512);

        // Off omits the field entirely.
        let client3 = make_client(&mock_server.uri(), test_or_config());
        client3
            .generate(&user_stack(), "", None, LlmGenerateOptions::default())
            .await
            .expect("generate");
        let reqs = mock_server.received_requests().await.expect("reqs");
        let body3: serde_json::Value = serde_json::from_slice(&reqs[2].body).expect("json");
        assert!(body3.get("reasoning").is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fallback_models_use_models_list_and_fallback_route() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse_body_ok()))
            .mount(&mock_server)
            .await;

        let mut or = test_or_config();
        or.fallback_models = vec!["fallback/model-2".into()];
        let client = make_client(&mock_server.uri(), or);
        client
            .generate(&user_stack(), "", None, LlmGenerateOptions::default())
            .await
            .expect("generate");
        let body = posted_body(&mock_server).await;
        assert_eq!(body["models"][0], "test/model-1");
        assert_eq!(body["models"][1], "fallback/model-2");
        assert_eq!(body["route"], "fallback");
    }

    fn sample_native_tools() -> Arc<Vec<OpenAiNativeTool>> {
        Arc::new(vec![OpenAiNativeTool::function(
            "memory:query".into(),
            "Search memories".into(),
            serde_json::json!({
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"],
                "additionalProperties": false
            }),
        )])
    }

    fn native_tool_options(choice: ToolChoice) -> LlmGenerateOptions {
        LlmGenerateOptions {
            native_tools: Some(sample_native_tools()),
            tool_choice: Some(choice),
            response_json_schema: Some(Arc::new(serde_json::json!({"type": "object"}))),
            ..Default::default()
        }
    }

    fn sse_tool_call_fragments() -> String {
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"memory:query\",\"arguments\":\"\"}}]}}]}\n\n\
         data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"query\\\":\"}}]}}]}\n\n\
         data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"foo\\\"}\"}}]}}]}\n\n\
         data: {\"choices\":[{\"delta\":{}}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":3,\"cost\":0.01}}\n\n\
         data: [DONE]\n\n"
            .to_string()
    }

    fn sse_reasoning_then_talk() -> String {
        "data: {\"choices\":[{\"delta\":{\"reasoning\":\"think first\"}}]}\n\n\
         data: {\"choices\":[{\"delta\":{\"content\":\"hello user\"}}]}\n\n\
         data: {\"choices\":[{\"delta\":{}}],\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":2}}\n\n\
         data: [DONE]\n\n"
            .to_string()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tools_attached_when_offered_subset_nonempty() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse_tool_call_fragments()))
            .mount(&mock_server)
            .await;

        let client = make_client(&mock_server.uri(), test_or_config());
        client
            .generate(
                &user_stack(),
                "",
                None,
                native_tool_options(ToolChoice::Auto),
            )
            .await
            .expect("generate");

        let body = posted_body(&mock_server).await;
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "memory:query");
        assert_eq!(body["tools"][0]["function"]["strict"], true);
        assert!(
            body.get("response_format").is_none(),
            "tools and response_format are mutually exclusive"
        );
        assert_eq!(body["provider"]["require_parameters"], true);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tool_choice_required_when_router_confident() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse_body_ok()))
            .mount(&mock_server)
            .await;

        let client = make_client(&mock_server.uri(), test_or_config());
        client
            .generate(
                &user_stack(),
                "",
                None,
                native_tool_options(ToolChoice::Required),
            )
            .await
            .expect("generate");

        let body = posted_body(&mock_server).await;
        assert_eq!(body["tool_choice"], "required");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn streamed_tool_call_fragments_assemble() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse_tool_call_fragments()))
            .mount(&mock_server)
            .await;

        let client = make_client(&mock_server.uri(), test_or_config());
        let result = client
            .generate(
                &user_stack(),
                "",
                None,
                native_tool_options(ToolChoice::Required),
            )
            .await
            .expect("generate");
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].id.as_deref(), Some("call_1"));
        assert_eq!(result.tool_calls[0].name, "memory:query");
        assert_eq!(result.tool_calls[0].arguments, r#"{"query":"foo"}"#);
        assert!(result.content.is_empty());
        assert_eq!(result.prompt_tokens, 5);
        assert_eq!(result.generated_tokens, 3);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reasoning_stays_out_of_content() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse_reasoning_then_talk()))
            .mount(&mock_server)
            .await;

        let client = make_client(&mock_server.uri(), test_or_config());
        let result = client
            .generate(&user_stack(), "", None, LlmGenerateOptions::default())
            .await
            .expect("generate");
        assert_eq!(result.content, "hello user");
        assert_eq!(result.reasoning, "think first");
        assert!(
            !result.content.contains("think"),
            "reasoning must not leak into content"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unsupported_tools_400_downgrades_to_response_format() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(move |req: &Request| {
                let body: serde_json::Value =
                    serde_json::from_slice(&req.body).expect("request json");
                if body.get("tools").is_some() {
                    ResponseTemplate::new(400)
                        .set_body_string(r#"{"error":{"message":"tools unsupported"}}"#)
                } else {
                    ResponseTemplate::new(200).set_body_string(sse_body_ok())
                }
            })
            .expect(3)
            .mount(&mock_server)
            .await;

        let client = make_client(&mock_server.uri(), test_or_config());
        let opts = native_tool_options(ToolChoice::Required);
        let result = client
            .generate(&user_stack(), "", None, opts.clone())
            .await
            .expect("generate after tools downgrade");
        assert_eq!(result.content, "Hello world");
        assert!(
            !client.native_tools_enabled.load(Ordering::SeqCst),
            "session must disable native tools"
        );

        client
            .generate(&user_stack(), "", None, opts)
            .await
            .expect("second generate stays on envelope");
        let reqs = mock_server.received_requests().await.expect("reqs");
        assert_eq!(reqs.len(), 3);
        let first: serde_json::Value = serde_json::from_slice(&reqs[0].body).expect("json");
        assert!(first.get("tools").is_some());
        let second: serde_json::Value = serde_json::from_slice(&reqs[1].body).expect("json");
        assert!(second.get("tools").is_none());
        assert_eq!(second["response_format"]["type"], "json_schema");
        let third: serde_json::Value = serde_json::from_slice(&reqs[2].body).expect("json");
        assert!(third.get("tools").is_none());
        assert_eq!(third["response_format"]["type"], "json_schema");
    }

    #[test]
    fn constructor_enforces_consent_gate() {
        let mut config = AppConfig::default();
        config.llm_backend = crate::config::LlmBackend::OpenRouter;
        config.openrouter = Some(OpenRouterConfig {
            model: "test/model".into(),
            consent_acknowledged: false,
            // PATH is always set — lets validation pass so we hit the consent check.
            api_key_env: "PATH".into(),
            ..Default::default()
        });
        let err = match OpenRouterClient::new(Arc::new(config)) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("consent gate must block construction"),
        };
        assert!(err.contains("consent"), "{err}");
    }
}
