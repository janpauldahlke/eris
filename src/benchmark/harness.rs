//! Benchmark harness for executing scenarios against the real orchestrator.

use crate::benchmark::suite::{Scenario, ScenarioResult, Step, SuccessCriteria};
use crate::benchmark::{CleanupReport, IsolationMode, QualityMetrics, SideEffectFilter};
use crate::benchmark::metrics::StepTiming;
use crate::engine::AnyEngine;
use crate::engine::Message;
use crate::executive::error::Result;
use crate::orchestrator::core::Orchestrator;
use crate::orchestrator::llm_support::json_envelope::parse_llm_response_protocol;
use crate::orchestrator::state::AgentState;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

pub struct BenchmarkHarness {
    isolation: super::BenchmarkIsolation,
    metrics: Arc<Mutex<QualityMetrics>>,
    _side_effect_filter: SideEffectFilter,
}

impl BenchmarkHarness {
    pub fn new(original_vault: &Path, isolation_mode: IsolationMode) -> Result<Self> {
        let isolation = super::BenchmarkIsolation::new(original_vault)?;
        let side_effect_filter = match isolation_mode {
            IsolationMode::Strict => SideEffectFilter::strict(),
            IsolationMode::Relaxed => SideEffectFilter::relaxed(),
            IsolationMode::Unsafe => SideEffectFilter::relaxed(),
        };
        Ok(Self {
            isolation,
            metrics: Arc::new(Mutex::new(QualityMetrics::default())),
            _side_effect_filter: side_effect_filter,
        })
    }

    pub fn temp_vault_path(&self) -> &Path {
        self.isolation.vault_root()
    }

    /// Run one scenario through the shared orchestrator (real LLM + tools).
    ///
    /// `scenario_timeout_floor_secs` comes from [`crate::config::AppConfig::benchmark_scenario_timeout_secs`].
    /// Effective deadline: `max(scenario.timeout_seconds, scenario_timeout_floor_secs)`.
    pub async fn run_scenario_with_orchestrator(
        &self,
        orchestrator: &mut Orchestrator<AnyEngine>,
        scenario: &Scenario,
        scenario_timeout_floor_secs: u64,
    ) -> Result<(ScenarioResult, Vec<StepTiming>)> {
        let timeout_secs = scenario
            .timeout_seconds
            .max(scenario_timeout_floor_secs)
            .max(1);
        let started = Instant::now();

        let executed = tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            Self::execute_scenario(orchestrator, scenario, started),
        )
        .await;

        let (result, timings) = match executed {
            Ok(pair) => pair,
            Err(_) => {
                tracing::warn!(
                    scenario = %scenario.name,
                    secs = timeout_secs,
                    "Benchmark scenario timed out"
                );
                let mut m = QualityMetrics::default();
                m.record_timeout();
                (
                    ScenarioResult {
                        scenario_name: scenario.name.clone(),
                        succeeded: false,
                        rounds_taken: 0,
                        max_rounds: scenario.steps.iter().map(|s| s.max_rounds).sum(),
                        steps_completed: 0,
                        total_steps: scenario.steps.len() as u32,
                        duration: started.elapsed(),
                        metrics: m,
                        error_message: Some(format!(
                            "Exceeded scenario timeout of {}s",
                            timeout_secs
                        )),
                    },
                    Vec::new(),
                )
            }
        };

        {
            let mut global = self.metrics.lock().await;
            global.merge(&result.metrics);
        }

        Ok((result, timings))
    }

    async fn execute_scenario(
        orchestrator: &mut Orchestrator<AnyEngine>,
        scenario: &Scenario,
        started: Instant,
    ) -> (ScenarioResult, Vec<StepTiming>) {
        let mut scenario_metrics = QualityMetrics::default();
        let mut step_timings = Vec::new();

        orchestrator.chat_stack.clear();
        orchestrator.state = AgentState::Idle;

        let mut rounds_taken: u32 = 0;
        let mut steps_completed: u32 = 0;
        let mut steps_tool_ok = true;
        let mut all_assistant_json_ok = true;
        let mut combined_user_visible = String::new();
        let mut last_user_visible = String::new();

        for step in &scenario.steps {
            let user_idx = orchestrator.chat_stack.len();
            orchestrator.chat_stack.push(Message {
                role: "user".to_string(),
                content: step.user_prompt.clone(),
            });
            orchestrator.state = AgentState::Chat;

            if let Err(e) = orchestrator.step(None).await {
                tracing::error!(
                    scenario = %scenario.name,
                    step = %step.description,
                    error = %e,
                    "Orchestrator step failed during benchmark"
                );
                return (
                    ScenarioResult {
                        scenario_name: scenario.name.clone(),
                        succeeded: false,
                        rounds_taken,
                        max_rounds: scenario.steps.iter().map(|s| s.max_rounds).sum(),
                        steps_completed,
                        total_steps: scenario.steps.len() as u32,
                        duration: started.elapsed(),
                        metrics: scenario_metrics,
                        error_message: Some(format!("{}", e)),
                    },
                    step_timings,
                );
            }

            step_timings.push(StepTiming {
                llm_ms: orchestrator.last_llm_ms,
                tool_ms: orchestrator.last_tool_ms,
                total_ms: orchestrator.last_total_ms,
            });

            rounds_taken = rounds_taken.saturating_add(orchestrator.tool_rounds as u32);

            let tail = if orchestrator.chat_stack.len() > user_idx {
                &orchestrator.chat_stack[user_idx + 1..]
            } else {
                &[][..]
            };

            let (tools_called, step_parse_ok, step_text, last_mu) =
                assistant_metrics_and_tools(tail, &mut scenario_metrics);
            all_assistant_json_ok &= step_parse_ok;
            combined_user_visible.push_str(&step_text);
            last_user_visible = last_mu;

            let step_ok = step_tools_satisfied(step, &tools_called, scenario);
            if !step_ok {
                steps_tool_ok = false;
                tracing::warn!(
                    scenario = %scenario.name,
                    step = %step.description,
                    expected = ?step.expected_tool_calls,
                    actual = ?tools_called,
                    "Benchmark step tool expectations not met"
                );
            }

            if step_ok {
                steps_completed += 1;
            }
        }

        let duration = started.elapsed();
        let criteria_ok = evaluate_success_criteria(
            scenario,
            steps_tool_ok,
            all_assistant_json_ok,
            &combined_user_visible,
            &last_user_visible,
        );

        let succeeded = steps_completed == scenario.steps.len() as u32 && criteria_ok;

        let error_message = if succeeded {
            None
        } else if !criteria_ok {
            Some("Scenario success criteria not met".into())
        } else {
            Some("One or more steps did not satisfy tool expectations".into())
        };

        (
            ScenarioResult {
                scenario_name: scenario.name.clone(),
                succeeded,
                rounds_taken,
                max_rounds: scenario.steps.iter().map(|s| s.max_rounds).sum(),
                steps_completed,
                total_steps: scenario.steps.len() as u32,
                duration,
                metrics: scenario_metrics,
                error_message,
            },
            step_timings,
        )
    }

    pub async fn metrics(&self) -> QualityMetrics {
        self.metrics.lock().await.clone()
    }

    pub async fn cleanup(&self) -> Result<CleanupReport> {
        Ok(self.isolation.cleanup_report())
    }
}

/// Parse assistant segments of `tail`, record JSON/tool metrics, return tool names and text.
fn assistant_metrics_and_tools(
    tail: &[Message],
    metrics: &mut QualityMetrics,
) -> (Vec<String>, bool, String, String) {
    let mut tools = Vec::new();
    let mut all_ok = true;
    let mut concat = String::new();
    let mut last_mu = String::new();

    for m in tail {
        if m.role != "assistant" {
            continue;
        }
        metrics.record_json_attempt();
        match parse_llm_response_protocol(&m.content) {
            Ok(resp) => {
                metrics.record_json_success();
                if let Some(mu) = &resp.message_to_user {
                    concat.push_str(mu);
                    concat.push(' ');
                    last_mu = mu.clone();
                }
                for tc in resp.tool_calls {
                    let name = tc.name.clone();
                    metrics.record_tool_attempt(&name);
                    metrics.record_tool_valid(&name);
                    tools.push(name);
                }
            }
            Err(_) => {
                all_ok = false;
            }
        }
    }

    (tools, all_ok, concat, last_mu)
}

fn step_tools_satisfied(step: &Step, actual_tools: &[String], scenario: &Scenario) -> bool {
    if step.expected_tool_calls.is_empty() {
        return true;
    }
    let actual: HashSet<&str> = actual_tools.iter().map(|s| s.as_str()).collect();
    match &scenario.success_criteria {
        SuccessCriteria::AnyToolCalled if step.expected_tool_calls.len() > 1 => step
            .expected_tool_calls
            .iter()
            .any(|e| actual.contains(e.as_str())),
        _ => step
            .expected_tool_calls
            .iter()
            .all(|e| actual.contains(e.as_str())),
    }
}

fn evaluate_success_criteria(
    scenario: &Scenario,
    steps_tool_ok: bool,
    all_assistant_json_ok: bool,
    combined_text: &str,
    last_message_to_user: &str,
) -> bool {
    if !steps_tool_ok {
        return false;
    }
    match &scenario.success_criteria {
        SuccessCriteria::AllToolsCalled | SuccessCriteria::AnyToolCalled => true,
        SuccessCriteria::Custom(_) => true,
        SuccessCriteria::ResponseContains(needle) => {
            combined_text.contains(needle.as_str()) || last_message_to_user.contains(needle.as_str())
        }
        SuccessCriteria::ValidJson => all_assistant_json_ok,
    }
}
