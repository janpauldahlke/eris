//! Instrumented engine for benchmark metrics collection.
//!
//! Wraps the real OllamaClient to capture detailed metrics during
//! benchmark execution including JSON parsing, recovery detection,
//! and timing data.

use crate::benchmark::metrics::{QualityMetrics, SpeedMetrics};
use crate::engine::ollama::OllamaClient;
use crate::engine::traits::{EngineResponse, LlmEngine, Message};
use crate::executive::error::Result;
use crate::orchestrator::llm_support::json_envelope::parse_llm_response_protocol;
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tracing;

/// Engine wrapper that captures benchmark metrics.
pub struct InstrumentedOllamaClient {
    /// Inner Ollama client.
    inner: OllamaClient,
    /// Shared metrics collector.
    metrics: Arc<Mutex<QualityMetrics>>,
    /// Shared speed metrics.
    speed_metrics: Arc<Mutex<SpeedMetrics>>,
}

impl InstrumentedOllamaClient {
    /// Create a new instrumented client wrapping an OllamaClient.
    pub fn new(inner: OllamaClient, metrics: Arc<Mutex<QualityMetrics>>) -> Self {
        Self {
            inner,
            metrics,
            speed_metrics: Arc::new(Mutex::new(SpeedMetrics::default())),
        }
    }

    /// Get the current speed metrics snapshot.
    pub async fn speed_metrics(&self) -> SpeedMetrics {
        self.speed_metrics.lock().await.clone()
    }

    /// Analyze an LLM response for quality metrics.
    async fn analyze_response(&self, response: &EngineResponse) {
        let mut metrics = self.metrics.lock().await;

        // Record JSON parse attempt
        metrics.record_json_attempt();

        // Try to parse as protocol JSON
        match parse_llm_response_protocol(&response.content) {
            Ok(_) => {
                metrics.record_json_success();
                tracing::debug!(
                    content_len = response.content.len(),
                    "JSON parse successful"
                );
            }
            Err(e) => {
                // This will trigger recovery - captured at orchestrator level
                tracing::debug!(
                    error = %e,
                    content_preview = %&response.content[..response.content.len().min(200)],
                    "JSON parse failed - recovery expected"
                );
            }
        }

        // Record speed metrics
        let mut speed = self.speed_metrics.lock().await;
        speed.prompt_tokens += response.prompt_tokens;
        speed.generated_tokens += response.generated_tokens;
    }
}

#[async_trait]
impl LlmEngine for InstrumentedOllamaClient {
    async fn generate(
        &self,
        stack: &[Message],
        available_tools_json: &str,
        stream_tx: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    ) -> Result<EngineResponse> {
        let start = Instant::now();

        tracing::debug!(
            message_count = stack.len(),
            has_stream = stream_tx.is_some(),
            "InstrumentedOllamaClient: generating"
        );

        // Call the real Ollama client
        let result = self
            .inner
            .generate(stack, available_tools_json, stream_tx)
            .await;

        let total_duration = start.elapsed();

        match &result {
            Ok(response) => {
                // Analyze the response for quality metrics
                self.analyze_response(response).await;

                // Record timing (approximation - full timing from Ollama response metadata)
                let mut speed = self.speed_metrics.lock().await;
                speed.total_duration += total_duration;

                tracing::debug!(
                    prompt_tokens = response.prompt_tokens,
                    generated_tokens = response.generated_tokens,
                    total_duration_ms = total_duration.as_millis(),
                    "InstrumentedOllamaClient: generation complete"
                );
            }
            Err(e) => {
                // Check if this is a timeout
                let error_str = e.to_string();
                if error_str.contains("timed out") || error_str.contains("timeout") {
                    let mut metrics = self.metrics.lock().await;
                    metrics.record_timeout();
                    tracing::warn!("InstrumentedOllamaClient: timeout detected");
                }

                tracing::error!(error = %e, "InstrumentedOllamaClient: generation failed");
            }
        }

        result
    }
}

/// Factory for creating instrumented engines.
pub struct InstrumentedEngineFactory;

impl InstrumentedEngineFactory {
    /// Wrap an existing OllamaClient with instrumentation.
    pub fn wrap(
        client: OllamaClient,
        metrics: Arc<Mutex<QualityMetrics>>,
    ) -> InstrumentedOllamaClient {
        InstrumentedOllamaClient::new(client, metrics)
    }
}

/// Helper to record recovery events from the orchestrator.
pub async fn record_recovery_attempt(
    metrics: &Arc<Mutex<QualityMetrics>>,
    succeeded: bool,
    failure_type: &str,
) {
    metrics.lock().await.record_recovery(succeeded);

    if succeeded {
        tracing::debug!(
            failure_type = %failure_type,
            "Recovery succeeded"
        );
    } else {
        tracing::warn!(
            failure_type = %failure_type,
            "Recovery failed"
        );
    }
}

/// Helper to record tool call validation.
pub async fn record_tool_validation(
    metrics: &Arc<Mutex<QualityMetrics>>,
    tool_name: &str,
    valid: bool,
    schema_errors: Option<Vec<String>>,
) {
    let mut m = metrics.lock().await;
    m.record_tool_attempt(tool_name);

    if valid {
        m.record_tool_valid(tool_name);
    } else if let Some(errors) = schema_errors {
        use crate::benchmark::metrics::{FailureAnalysis, FailureType};
        use chrono::Utc;

        m.add_failure(FailureAnalysis {
            scenario: "tool_validation".to_string(),
            failure_type: FailureType::SchemaValidation,
            raw_llm_output: format!("Tool: {} Errors: {:?}", tool_name, errors),
            expected_tool: None,
            actual_tool: Some(tool_name.to_string()),
            schema_errors: errors,
            parse_error: None,
            timestamp: Utc::now(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use std::sync::Arc;

    fn create_test_client() -> (OllamaClient, Arc<Mutex<QualityMetrics>>) {
        use ollama_rs::Ollama;

        let config = Arc::new(AppConfig::default());
        let ollama = Ollama::new("http://localhost".to_string(), 11434);
        let client = OllamaClient::new(ollama, config);
        let metrics = Arc::new(Mutex::new(QualityMetrics::default()));

        (client, metrics)
    }

    #[tokio::test]
    async fn instrumented_client_records_metrics() {
        let (client, metrics) = create_test_client();
        let _instrumented = InstrumentedOllamaClient::new(client, metrics.clone());

        // Simulate metrics recording
        {
            let mut m = metrics.lock().await;
            m.record_json_attempt();
            m.record_json_success();
            m.record_tool_attempt("memory:stage");
            m.record_tool_valid("memory:stage");
        }

        let snapshot = metrics.lock().await.clone();
        assert_eq!(snapshot.json_parse_attempts, 1);
        assert_eq!(snapshot.json_parse_successes, 1);
        assert_eq!(snapshot.tool_calls_attempted, 1);
        assert_eq!(snapshot.tool_calls_valid, 1);
    }

    #[test]
    fn analyze_response_detects_valid_json() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (client, metrics) = create_test_client();
            let instrumented = InstrumentedOllamaClient::new(client, metrics.clone());

            let response = EngineResponse {
                content: r#"{"thought":"test","status":"Idle","message_to_user":"hi","tool_calls":[]}"#.to_string(),
                prompt_tokens: 10,
                generated_tokens: 5,
                generation_ms: 0,
            };

            instrumented.analyze_response(&response).await;

            let m = metrics.lock().await;
            assert_eq!(m.json_parse_attempts, 1);
            assert_eq!(m.json_parse_successes, 1);
        });
    }

    #[test]
    fn analyze_response_detects_invalid_json() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (client, metrics) = create_test_client();
            let instrumented = InstrumentedOllamaClient::new(client, metrics.clone());

            let response = EngineResponse {
                content: "not valid json".to_string(),
                prompt_tokens: 10,
                generated_tokens: 5,
                generation_ms: 0,
            };

            instrumented.analyze_response(&response).await;

            let m = metrics.lock().await;
            assert_eq!(m.json_parse_attempts, 1);
            assert_eq!(m.json_parse_successes, 0);
        });
    }

    #[tokio::test]
    async fn record_recovery_updates_metrics() {
        let metrics = Arc::new(Mutex::new(QualityMetrics::default()));

        record_recovery_attempt(&metrics, true, "json_parse").await;
        record_recovery_attempt(&metrics, false, "schema_validation").await;

        let m = metrics.lock().await;
        assert_eq!(m.recovery_triggers, 2);
        assert_eq!(m.recovery_successes, 1);
    }

    #[tokio::test]
    async fn record_tool_validation_updates_metrics() {
        let metrics = Arc::new(Mutex::new(QualityMetrics::default()));

        record_tool_validation(&metrics, "memory:stage", true, None).await;
        record_tool_validation(&metrics, "vault:read", false, Some(vec!["missing path".to_string()])).await;

        let m = metrics.lock().await;
        assert_eq!(m.tool_calls_attempted, 2);
        assert_eq!(m.tool_calls_valid, 1);
        assert_eq!(m.failure_analyses.len(), 1);
    }
}
