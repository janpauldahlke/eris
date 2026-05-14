//! One-shot latency samples for benchmark console reports.
//!
//! Scenario harness steps do not call the LLM yet; this probe captures **model-level**
//! prompt/gen timings and token counts from a minimal completion. True streaming TTFT is not
//! measured here (would require the streaming API and first-chunk timestamps).

use crate::benchmark::metrics::SpeedMetrics;
use crate::engine::llama_cpp::LlamaCppClient;
use crate::engine::ollama::OllamaClient;
use crate::engine::{LlmEngine, LlmGenerateOptions, Message};
use crate::executive::error::{FcpError, Result};
use ollama_rs::generation::chat::request::ChatMessageRequest;
use ollama_rs::generation::chat::ChatMessage;
use ollama_rs::models::ModelOptions;
use std::time::Duration;

/// Single minimal chat to populate [`SpeedMetrics`] from Ollama-reported nanosecond timings.
pub async fn probe_ollama_chat_latency(client: &OllamaClient) -> Result<SpeedMetrics> {
    let gen_options = ModelOptions::default().num_ctx(client.config.num_ctx as u64);

    let request = ChatMessageRequest::new(
        client.config.model_name.clone(),
        vec![ChatMessage::user(
            "Reply with exactly one word: pong.".to_string(),
        )],
    )
    .options(gen_options);

    let response = client
        .client
        .send_chat_messages(request)
        .await
        .map_err(|e| FcpError::NetworkFault(format!("Benchmark speed probe: Ollama chat failed: {e}")))?;

    let fd = response.final_data.ok_or_else(|| {
        FcpError::EngineFault(
            "Benchmark speed probe: Ollama response missing final_data timings".into(),
        )
    })?;

    Ok(SpeedMetrics {
        prompt_tokens: fd.prompt_eval_count as usize,
        generated_tokens: fd.eval_count as usize,
        prompt_eval_duration: Duration::from_nanos(fd.prompt_eval_duration),
        eval_duration: Duration::from_nanos(fd.eval_duration),
        time_to_first_token: Duration::ZERO,
        total_duration: Duration::from_nanos(fd.total_duration),
    })
}

/// Minimal chat completion against llama-server (no GBNF on this client — probe only).
///
/// Uses [`EngineResponse::generation_ms`] and usage for throughput; prompt vs gen wall split is approximate.
pub async fn probe_llamacpp_chat_latency(client: &LlamaCppClient) -> Result<SpeedMetrics> {
    let stack = vec![Message {
        role: "user".to_string(),
        content: "Reply with exactly one word: pong.".to_string(),
    }];
    let wall_start = std::time::Instant::now();
    let resp = client
        .generate(&stack, "[]", None, LlmGenerateOptions::default())
        .await
        .map_err(|e| {
            FcpError::NetworkFault(format!("Benchmark speed probe: llama.cpp chat failed: {e}"))
        })?;
    let wall = wall_start.elapsed();
    let gen_ms = resp.generation_ms.max(1);
    let eval_duration = Duration::from_millis(gen_ms);
    let prompt_eval_duration = wall.checked_sub(eval_duration).unwrap_or(Duration::ZERO);

    Ok(SpeedMetrics {
        prompt_tokens: resp.prompt_tokens,
        generated_tokens: resp.generated_tokens,
        prompt_eval_duration,
        eval_duration,
        time_to_first_token: Duration::ZERO,
        total_duration: wall,
    })
}
