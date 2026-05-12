//! Quality metrics collection for benchmarking.
//!
//! Captures metrics beyond raw speed:
//! - JSON protocol adherence
//! - Recovery success rates
//! - Tool selection accuracy
//! - Failure analysis with raw model outputs

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// Quality metrics for a benchmark run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QualityMetrics {
    /// Number of JSON parse attempts.
    pub json_parse_attempts: u32,
    /// Number of successful JSON parses.
    pub json_parse_successes: u32,
    /// Number of recovery triggers (schema/parse failures).
    pub recovery_triggers: u32,
    /// Number of successful recoveries.
    pub recovery_successes: u32,
    /// Number of tool call attempts.
    pub tool_calls_attempted: u32,
    /// Number of valid tool calls (passed Gatekeeper).
    pub tool_calls_valid: u32,
    /// Number of timeouts.
    pub timeout_count: u32,
    /// Detailed failure analyses.
    pub failure_analyses: Vec<FailureAnalysis>,
    /// Tool-specific metrics.
    pub tool_metrics: HashMap<String, ToolMetrics>,
    /// Scenario-specific results.
    pub scenario_results: Vec<ScenarioResult>,
}

impl QualityMetrics {
    /// Calculate JSON parse success rate.
    pub fn json_success_rate(&self) -> f64 {
        if self.json_parse_attempts == 0 {
            return 0.0;
        }
        (self.json_parse_successes as f64 / self.json_parse_attempts as f64) * 100.0
    }

    /// Calculate recovery success rate.
    pub fn recovery_success_rate(&self) -> f64 {
        if self.recovery_triggers == 0 {
            return 100.0;
        }
        (self.recovery_successes as f64 / self.recovery_triggers as f64) * 100.0
    }

    /// Calculate tool validation success rate.
    pub fn tool_valid_rate(&self) -> f64 {
        if self.tool_calls_attempted == 0 {
            return 0.0;
        }
        (self.tool_calls_valid as f64 / self.tool_calls_attempted as f64) * 100.0
    }

    /// Calculate timeout rate.
    pub fn timeout_rate(&self) -> f64 {
        let total_attempts = self.json_parse_attempts + self.recovery_triggers;
        if total_attempts == 0 {
            return 0.0;
        }
        (self.timeout_count as f64 / total_attempts as f64) * 100.0
    }

    /// Record a JSON parse attempt.
    pub fn record_json_attempt(&mut self) {
        self.json_parse_attempts += 1;
    }

    /// Record a successful JSON parse.
    pub fn record_json_success(&mut self) {
        self.json_parse_successes += 1;
    }

    /// Record a recovery trigger and whether it succeeded.
    pub fn record_recovery(&mut self, succeeded: bool) {
        self.recovery_triggers += 1;
        if succeeded {
            self.recovery_successes += 1;
        }
    }

    /// Record a tool call attempt.
    pub fn record_tool_attempt(&mut self, tool_name: &str) {
        self.tool_calls_attempted += 1;
        self.tool_metrics
            .entry(tool_name.to_string())
            .or_default()
            .attempts += 1;
    }

    /// Record a valid tool call.
    pub fn record_tool_valid(&mut self, tool_name: &str) {
        self.tool_calls_valid += 1;
        self.tool_metrics
            .entry(tool_name.to_string())
            .or_default()
            .valid += 1;
    }

    /// Record a timeout.
    pub fn record_timeout(&mut self) {
        self.timeout_count += 1;
    }

    /// Add a failure analysis.
    pub fn add_failure(&mut self, analysis: FailureAnalysis) {
        self.failure_analyses.push(analysis);
    }

    /// Add a scenario result.
    pub fn add_scenario_result(&mut self, result: ScenarioResult) {
        self.scenario_results.push(result);
    }

    /// Get overall quality score (0-100).
    /// Weighted combination of JSON success, recovery success, and tool validity.
    pub fn overall_quality_score(&self) -> f64 {
        let json_weight = 0.4;
        let recovery_weight = 0.3;
        let tool_weight = 0.3;

        self.json_success_rate() * json_weight
            + self.recovery_success_rate() * recovery_weight
            + self.tool_valid_rate() * tool_weight
    }

    /// Merge another metrics instance into this one.
    pub fn merge(&mut self, other: &QualityMetrics) {
        self.json_parse_attempts += other.json_parse_attempts;
        self.json_parse_successes += other.json_parse_successes;
        self.recovery_triggers += other.recovery_triggers;
        self.recovery_successes += other.recovery_successes;
        self.tool_calls_attempted += other.tool_calls_attempted;
        self.tool_calls_valid += other.tool_calls_valid;
        self.timeout_count += other.timeout_count;
        self.failure_analyses.extend(other.failure_analyses.clone());

        for (tool, metrics) in &other.tool_metrics {
            let entry = self.tool_metrics.entry(tool.clone()).or_default();
            entry.attempts += metrics.attempts;
            entry.valid += metrics.valid;
        }

        self.scenario_results.extend(other.scenario_results.clone());
    }
}

/// Metrics for a specific tool.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolMetrics {
    pub attempts: u32,
    pub valid: u32,
}

impl ToolMetrics {
    /// Calculate validity rate for this tool.
    pub fn valid_rate(&self) -> f64 {
        if self.attempts == 0 {
            return 0.0;
        }
        (self.valid as f64 / self.attempts as f64) * 100.0
    }
}

/// Analysis of a specific failure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureAnalysis {
    /// Scenario name where failure occurred.
    pub scenario: String,
    /// Type of failure.
    pub failure_type: FailureType,
    /// Raw LLM output that caused the failure.
    pub raw_llm_output: String,
    /// Expected tool (if applicable).
    pub expected_tool: Option<String>,
    /// Actual tool attempted (if applicable).
    pub actual_tool: Option<String>,
    /// Schema validation errors.
    pub schema_errors: Vec<String>,
    /// Parse error details.
    pub parse_error: Option<String>,
    /// Timestamp of failure.
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Types of failures that can occur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureType {
    /// JSON parsing failed.
    JsonParse,
    /// Schema validation failed.
    SchemaValidation,
    /// Tool not found in registry.
    ToolNotFound,
    /// Tool not authorized in current state.
    ToolNotAuthorized,
    /// Wrong tool selected for scenario.
    WrongTool,
    /// Timeout during generation.
    Timeout,
    /// Recovery failed after multiple attempts.
    RecoveryFailed,
    /// Scenario-specific validation failed.
    ScenarioValidation,
    /// Other error.
    Other,
}

mod duration_ms {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.as_millis().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(millis))
    }
}

/// Result of a single scenario execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioResult {
    /// Scenario name.
    pub scenario_name: String,
    /// Whether the scenario succeeded.
    pub succeeded: bool,
    /// Number of tool rounds taken.
    pub rounds_taken: u32,
    /// Maximum allowed rounds.
    pub max_rounds: u32,
    /// Steps completed (if applicable).
    pub steps_completed: u32,
    /// Total steps expected.
    pub total_steps: u32,
    /// Duration of scenario execution.
    #[serde(with = "duration_ms")]
    pub duration: Duration,
    /// Quality metrics for this scenario.
    pub metrics: QualityMetrics,
    /// Error message if failed.
    pub error_message: Option<String>,
}

/// Speed metrics for performance measurement.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpeedMetrics {
    /// Prompt evaluation tokens.
    pub prompt_tokens: usize,
    /// Generated tokens.
    pub generated_tokens: usize,
    /// Prompt evaluation time.
    pub prompt_eval_duration: Duration,
    /// Generation time.
    pub eval_duration: Duration,
    /// Total time to first token.
    pub time_to_first_token: Duration,
    /// Total request time.
    pub total_duration: Duration,
}

/// One completed user turn: values mirror [`crate::orchestrator::core::Orchestrator`] after `step()` returns
/// (`last_llm_ms` sums intra-turn LLM generations; `last_total_ms` is wall time for the whole turn).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StepTiming {
    pub llm_ms: u64,
    pub tool_ms: u64,
    pub total_ms: u64,
}

/// Mean orchestrator timings over **user steps from scenarios that passed only** (survivorship bias possible).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SuiteSpeedAggregate {
    /// Steps included in the means (`contributing_scenarios` × steps per scenario, varying).
    pub step_samples: u32,
    /// Scenarios that passed and contributed at least one step sample.
    pub contributing_scenarios: u32,
    pub mean_llm_ms: f64,
    pub mean_tool_ms: f64,
    pub mean_total_ms: f64,
}

impl SuiteSpeedAggregate {
    /// Build from per-step samples (only from successful scenarios).
    pub fn from_step_samples(samples: &[StepTiming], contributing_scenarios: u32) -> Self {
        if samples.is_empty() {
            return Self::default();
        }
        let n = samples.len() as f64;
        let sum_llm: u64 = samples.iter().map(|s| s.llm_ms).sum();
        let sum_tool: u64 = samples.iter().map(|s| s.tool_ms).sum();
        let sum_total: u64 = samples.iter().map(|s| s.total_ms).sum();
        Self {
            step_samples: samples.len() as u32,
            contributing_scenarios,
            mean_llm_ms: sum_llm as f64 / n,
            mean_tool_ms: sum_tool as f64 / n,
            mean_total_ms: sum_total as f64 / n,
        }
    }
}

impl SpeedMetrics {
    /// Calculate prompt throughput (tokens/second).
    pub fn prompt_throughput(&self) -> f64 {
        let secs = self.prompt_eval_duration.as_secs_f64();
        if secs == 0.0 {
            return 0.0;
        }
        self.prompt_tokens as f64 / secs
    }

    /// Calculate generation throughput (tokens/second).
    pub fn generation_throughput(&self) -> f64 {
        let secs = self.eval_duration.as_secs_f64();
        if secs == 0.0 {
            return 0.0;
        }
        self.generated_tokens as f64 / secs
    }

    /// Calculate total throughput.
    pub fn total_throughput(&self) -> f64 {
        let total_tokens = self.prompt_tokens + self.generated_tokens;
        let secs = self.total_duration.as_secs_f64();
        if secs == 0.0 {
            return 0.0;
        }
        total_tokens as f64 / secs
    }
}

/// Combined metrics for a complete benchmark run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkReport {
    /// Benchmark run ID.
    pub run_id: String,
    /// Timestamp of run.
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Model name.
    pub model_name: String,
    /// Suite name (quick, standard, comprehensive).
    pub suite: String,
    /// Quality metrics.
    pub quality: QualityMetrics,
    /// Single minimal chat probe (Ollama: tok/s from `final_data`; LlamaCpp: from engine response + wall clock).
    pub speed: SpeedMetrics,
    /// Mean orchestrator timing per user step, **successful scenarios only** (see [`SuiteSpeedAggregate`]).
    #[serde(default)]
    pub suite_speed: SuiteSpeedAggregate,
    /// Isolation mode used.
    pub isolation_mode: String,
    /// Cleanup report.
    pub cleanup_report: CleanupConfirmation,
}

/// Confirmation that cleanup was successful.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CleanupConfirmation {
    /// Whether temp vault was cleaned up.
    pub temp_vault_cleaned: bool,
    /// Whether Qdrant collection was removed.
    pub qdrant_collection_removed: bool,
    /// Number of staged memories removed.
    pub staged_memories_removed: usize,
    /// Number of ephemeral entries removed.
    pub ephemeral_entries_removed: usize,
    /// Any failures during cleanup.
    pub cleanup_failures: Vec<String>,
}

impl CleanupConfirmation {
    /// Check if all cleanup succeeded.
    pub fn all_cleaned(&self) -> bool {
        self.temp_vault_cleaned
            && self.qdrant_collection_removed
            && self.cleanup_failures.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_metrics_calculates_rates() {
        let mut metrics = QualityMetrics::default();

        metrics.record_json_attempt();
        metrics.record_json_attempt();
        metrics.record_json_success();

        assert_eq!(metrics.json_success_rate(), 50.0);
    }

    #[test]
    fn recovery_rate_calculation() {
        let mut metrics = QualityMetrics::default();

        metrics.record_recovery(false);
        metrics.record_recovery(true);
        metrics.record_recovery(true);

        let rate = metrics.recovery_success_rate();
        assert!((rate - 66.66666666666667).abs() < 0.0001, "Expected ~66.67%, got {}", rate);
    }

    #[test]
    fn overall_quality_score_weights_correctly() {
        let mut metrics = QualityMetrics::default();

        // JSON: 100% (weight 0.4) = 40
        metrics.json_parse_attempts = 10;
        metrics.json_parse_successes = 10;

        // Recovery: 50% (weight 0.3) = 15
        metrics.recovery_triggers = 2;
        metrics.recovery_successes = 1;

        // Tools: 100% (weight 0.3) = 30
        metrics.tool_calls_attempted = 5;
        metrics.tool_calls_valid = 5;

        // Expected: 40 + 15 + 30 = 85
        assert!((metrics.overall_quality_score() - 85.0).abs() < 0.01);
    }

    #[test]
    fn speed_metrics_calculates_throughput() {
        let speed = SpeedMetrics {
            prompt_tokens: 100,
            generated_tokens: 50,
            prompt_eval_duration: Duration::from_secs(1),
            eval_duration: Duration::from_secs(2),
            total_duration: Duration::from_secs(3),
            ..Default::default()
        };

        assert_eq!(speed.prompt_throughput(), 100.0);
        assert_eq!(speed.generation_throughput(), 25.0);
        assert_eq!(speed.total_throughput(), 50.0);
    }

    #[test]
    fn metrics_merge_combines_correctly() {
        let mut m1 = QualityMetrics::default();
        m1.json_parse_attempts = 10;
        m1.json_parse_successes = 8;
        m1.record_tool_attempt("test");
        m1.record_tool_valid("test");

        let mut m2 = QualityMetrics::default();
        m2.json_parse_attempts = 5;
        m2.json_parse_successes = 5;
        m2.record_tool_attempt("test");

        m1.merge(&m2);

        assert_eq!(m1.json_parse_attempts, 15);
        assert_eq!(m1.json_parse_successes, 13);
        assert_eq!(m1.tool_metrics["test"].attempts, 2);
        assert_eq!(m1.tool_metrics["test"].valid, 1);
    }
}
