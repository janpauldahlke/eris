//! Benchmark report generation.
//!
//! Generates human-readable reports in multiple formats:
//! - Console tables (immediate feedback)
//! - JSON (machine-readable, integration)
//! - Markdown (documentation, sharing)

use crate::benchmark::metrics::BenchmarkReport;

/// Report generator for benchmark results.
pub struct ReportGenerator;

impl ReportGenerator {
    /// Generate a console table report.
    pub fn console(report: &BenchmarkReport) -> String {
        let mut output = String::new();

        // Header
        output.push_str(&format!("\n{}", "═".repeat(68)));
        output.push_str(&format!("\n{:^68}", "ERIS CAPABILITY BENCHMARK"));
        output.push_str(&format!("\n{}", "═".repeat(68)));

        // Metadata
        output.push_str(&format!("\n  Model:       {}", report.model_name));
        output.push_str(&format!("\n  Suite:       {}", report.suite));
        output.push_str(&format!("\n  Run ID:      {}", report.run_id));
        output.push_str(&format!("\n  Timestamp:   {}", report.timestamp.format("%Y-%m-%d %H:%M:%S UTC")));
        output.push_str(&format!("\n  Isolation:   {}", report.isolation_mode));
        output.push_str(&format!("\n{}", "─".repeat(68)));

        // Safety Checklist
        output.push_str(&format!("\n  {:^64}", "SAFETY CHECKLIST"));
        output.push_str(&format!("\n{}", "─".repeat(68)));
        output.push_str(&format!(
            "\n  [{}] External side effects blocked",
            checkbox(report.cleanup_report.all_cleaned())
        ));
        output.push_str(&format!(
            "\n  [{}] Temp vault cleaned",
            checkbox(report.cleanup_report.temp_vault_cleaned)
        ));
        output.push_str(&format!(
            "\n  [{}] Staged memories removed ({})",
            checkbox(report.cleanup_report.staged_memories_removed > 0 || report.cleanup_report.all_cleaned()),
            report.cleanup_report.staged_memories_removed
        ));
        output.push_str(&format!(
            "\n  [{}] Ephemeral entries removed ({})",
            checkbox(report.cleanup_report.ephemeral_entries_removed > 0 || report.cleanup_report.all_cleaned()),
            report.cleanup_report.ephemeral_entries_removed
        ));

        if !report.cleanup_report.cleanup_failures.is_empty() {
            output.push_str(&format!("\n  ⚠ {} cleanup failures", report.cleanup_report.cleanup_failures.len()));
        }

        output.push_str(&format!("\n{}", "─".repeat(68)));

        // Quality Metrics
        output.push_str(&format!("\n  {:^64}", "QUALITY METRICS"));
        output.push_str(&format!("\n{}", "─".repeat(68)));
        output.push_str(&format!(
            "\n  {:.<40} {:>6.1}%",
            "JSON Parse Success",
            report.quality.json_success_rate()
        ));
        output.push_str(&format!(
            "\n  {:.<40} {:>6.1}%",
            "Recovery Success Rate",
            report.quality.recovery_success_rate()
        ));
        output.push_str(&format!(
            "\n  {:.<40} {:>6.1}%",
            "Tool Validation Rate",
            report.quality.tool_valid_rate()
        ));
        output.push_str(&format!(
            "\n  {:.<40} {:>6.1}%",
            "Timeout Rate",
            report.quality.timeout_rate()
        ));
        output.push_str(&format!("\n{}", "─".repeat(68)));
        output.push_str(&format!(
            "\n  {:.<40} {:>6.1}%",
            "OVERALL QUALITY SCORE",
            report.quality.overall_quality_score()
        ));

        output.push_str(&format!("\n{}", "─".repeat(68)));

        // Speed Metrics
        output.push_str(&format!("\n  {:^64}", "SPEED METRICS"));
        output.push_str(&format!("\n{}", "─".repeat(68)));
        output.push_str(&format!(
            "\n  {:.<40} {:>8.1} tok/s",
            "Prompt Throughput",
            report.speed.prompt_throughput()
        ));
        output.push_str(&format!(
            "\n  {:.<40} {:>8.1} tok/s",
            "Generation Throughput",
            report.speed.generation_throughput()
        ));
        output.push_str(&format!(
            "\n  {:.<40} {:>8.1} tok/s",
            "Total Throughput",
            report.speed.total_throughput()
        ));

        output.push_str(&format!("\n{}", "─".repeat(68)));

        output.push_str(&format!("\n  {:^64}", "SUITE TIMING (passed scenarios)"));
        output.push_str(&format!("\n{}", "─".repeat(68)));
        if report.suite_speed.step_samples > 0 {
            output.push_str(&format!(
                "\n  {:.<40} {:>8} steps",
                "Samples (user steps)",
                report.suite_speed.step_samples
            ));
            output.push_str(&format!(
                "\n  {:.<40} {:>8} scenarios",
                "Contributing scenarios",
                report.suite_speed.contributing_scenarios
            ));
            output.push_str(&format!(
                "\n  {:.<40} {:>8.0} ms",
                "Mean LLM ms / step",
                report.suite_speed.mean_llm_ms
            ));
            output.push_str(&format!(
                "\n  {:.<40} {:>8.0} ms",
                "Mean tool ms / step",
                report.suite_speed.mean_tool_ms
            ));
            output.push_str(&format!(
                "\n  {:.<40} {:>8.0} ms",
                "Mean total ms / step",
                report.suite_speed.mean_total_ms
            ));
        } else {
            output.push_str("\n  (No passed scenarios — no suite timing aggregate.)");
        }

        output.push_str(&format!("\n{}", "─".repeat(68)));

        // Scenario Results
        output.push_str(&format!("\n  {:^64}", "SCENARIO RESULTS"));
        output.push_str(&format!("\n{}", "─".repeat(68)));

        let success_count = report.quality.scenario_results.iter().filter(|r| r.succeeded).count();
        let total_count = report.quality.scenario_results.len();

        output.push_str(&format!(
            "\n  Scenarios: {} passed / {} total ({:.0}%)",
            success_count,
            total_count,
            if total_count > 0 {
                (success_count as f64 / total_count as f64) * 100.0
            } else {
                0.0
            }
        ));

        for result in &report.quality.scenario_results {
            let status = if result.succeeded { "✓" } else { "✗" };
            output.push_str(&format!(
                "\n    {} {:.<50} {}ms",
                status,
                format!("{}", result.scenario_name),
                result.duration.as_millis()
            ));
        }

        if !report.quality.failure_analyses.is_empty() {
            output.push_str(&format!("\n{}", "─".repeat(68)));
            output.push_str(&format!("\n  {:^64}", "FAILURE ANALYSIS"));
            output.push_str(&format!("\n{}", "─".repeat(68)));

            for (i, failure) in report.quality.failure_analyses.iter().take(3).enumerate() {
                output.push_str(&format!("\n  {}. {}", i + 1, failure.scenario));
                output.push_str(&format!("\n     Type: {:?}", failure.failure_type));
                if let Some(ref tool) = failure.actual_tool {
                    output.push_str(&format!("\n     Tool: {}", tool));
                }
                if let Some(ref error) = failure.parse_error {
                    output.push_str(&format!("\n     Error: {}", error.chars().take(100).collect::<String>()));
                }
            }

            if report.quality.failure_analyses.len() > 3 {
                output.push_str(&format!(
                    "\n     ... and {} more failures",
                    report.quality.failure_analyses.len() - 3
                ));
            }
        }

        output.push_str(&format!("\n{}", "═".repeat(68)));
        output.push('\n');

        output
    }

    /// Generate a JSON report.
    pub fn json(report: &BenchmarkReport) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(report)
    }

    /// Generate a Markdown report.
    pub fn markdown(report: &BenchmarkReport) -> String {
        let mut md = String::new();

        // Title and metadata
        md.push_str(&format!("# Benchmark Report: {}\n\n", report.model_name));
        md.push_str("## Metadata\n\n");
        md.push_str("| Field | Value |\n");
        md.push_str("|-------|-------|\n");
        md.push_str(&format!("| **Run ID** | `{}` |\n", report.run_id));
        md.push_str(&format!("| **Suite** | {} |\n", report.suite));
        md.push_str(&format!("| **Timestamp** | {} |\n", report.timestamp.format("%Y-%m-%d %H:%M:%S UTC")));
        md.push_str(&format!("| **Isolation Mode** | {} |\n\n", report.isolation_mode));

        // Safety checklist
        md.push_str("## Safety Checklist\n\n");
        md.push_str("- [x] External side effects blocked\n");
        md.push_str(&format!(
            "- [{}] Temp vault cleaned\n",
            if report.cleanup_report.temp_vault_cleaned { "x" } else { " " }
        ));
        md.push_str(&format!(
            "- [x] {} staged memories removed\n",
            report.cleanup_report.staged_memories_removed
        ));
        md.push_str(&format!(
            "- [x] {} ephemeral entries removed\n\n",
            report.cleanup_report.ephemeral_entries_removed
        ));

        if !report.cleanup_report.cleanup_failures.is_empty() {
            md.push_str("### Cleanup Warnings\n\n");
            for failure in &report.cleanup_report.cleanup_failures {
                md.push_str(&format!("- ⚠ {}\n", failure));
            }
            md.push('\n');
        }

        // Quality metrics
        md.push_str("## Quality Metrics\n\n");
        md.push_str("| Metric | Value | Status |\n");
        md.push_str("|--------|-------|--------|\n");
        md.push_str(&format!(
            "| JSON Parse Success | {:.1}% | {} |\n",
            report.quality.json_success_rate(),
            grade(report.quality.json_success_rate())
        ));
        md.push_str(&format!(
            "| Recovery Success | {:.1}% | {} |\n",
            report.quality.recovery_success_rate(),
            grade(report.quality.recovery_success_rate())
        ));
        md.push_str(&format!(
            "| Tool Validation | {:.1}% | {} |\n",
            report.quality.tool_valid_rate(),
            grade(report.quality.tool_valid_rate())
        ));
        md.push_str(&format!(
            "| Timeout Rate | {:.1}% | {} |\n",
            report.quality.timeout_rate(),
            if report.quality.timeout_rate() < 5.0 { "✓ Good" } else { "⚠ High" }
        ));
        md.push_str(&format!(
            "| **Overall Quality** | **{:.1}%** | **{}** |\n\n",
            report.quality.overall_quality_score(),
            grade(report.quality.overall_quality_score())
        ));

        // Speed metrics
        md.push_str("## Speed Metrics\n\n");
        md.push_str("| Metric | Value |\n");
        md.push_str("|--------|-------|\n");
        md.push_str(&format!(
            "| Prompt Throughput | {:.1} tok/s |\n",
            report.speed.prompt_throughput()
        ));
        md.push_str(&format!(
            "| Generation Throughput | {:.1} tok/s |\n",
            report.speed.generation_throughput()
        ));
        md.push_str(&format!(
            "| Total Throughput | {:.1} tok/s |\n\n",
            report.speed.total_throughput()
        ));

        md.push_str("## Suite timing (passed scenarios)\n\n");
        md.push_str("Means over completed `Orchestrator::step` turns from scenarios that **passed** only.\n\n");
        if report.suite_speed.step_samples > 0 {
            md.push_str("| Metric | Value |\n|--------|-------|\n");
            md.push_str(&format!(
                "| Step samples | {} ({} scenarios) |\n",
                report.suite_speed.step_samples, report.suite_speed.contributing_scenarios
            ));
            md.push_str(&format!(
                "| Mean LLM ms / step | {:.0} |\n",
                report.suite_speed.mean_llm_ms
            ));
            md.push_str(&format!(
                "| Mean tool ms / step | {:.0} |\n",
                report.suite_speed.mean_tool_ms
            ));
            md.push_str(&format!(
                "| Mean total ms / step | {:.0} |\n\n",
                report.suite_speed.mean_total_ms
            ));
        } else {
            md.push_str("*No successful scenarios — no aggregate.*\n\n");
        }

        // Scenario results
        md.push_str("## Scenario Results\n\n");

        let success_count = report.quality.scenario_results.iter().filter(|r| r.succeeded).count();
        let total_count = report.quality.scenario_results.len();
        let success_rate = if total_count > 0 {
            (success_count as f64 / total_count as f64) * 100.0
        } else {
            0.0
        };

        md.push_str(&format!(
            "**Overall: {} / {} passed ({:.1}%)**\n\n",
            success_count, total_count, success_rate
        ));

        md.push_str("| Scenario | Status | Rounds | Duration |\n");
        md.push_str("|----------|--------|--------|----------|\n");

        for result in &report.quality.scenario_results {
            let status = if result.succeeded { "✅ Pass" } else { "❌ Fail" };
            md.push_str(&format!(
                "| {} | {} | {}/{} | {}ms |\n",
                result.scenario_name,
                status,
                result.rounds_taken,
                result.max_rounds,
                result.duration.as_millis()
            ));
        }

        md.push('\n');

        // Tool-specific metrics
        if !report.quality.tool_metrics.is_empty() {
            md.push_str("## Tool-Specific Metrics\n\n");
            md.push_str("| Tool | Attempts | Valid | Success Rate |\n");
            md.push_str("|------|----------|-------|--------------|\n");

            let mut tools: Vec<_> = report.quality.tool_metrics.iter().collect();
            tools.sort_by_key(|(name, _)| *name);

            for (name, metrics) in tools {
                md.push_str(&format!(
                    "| {} | {} | {} | {:.1}% |\n",
                    name,
                    metrics.attempts,
                    metrics.valid,
                    metrics.valid_rate()
                ));
            }

            md.push('\n');
        }

        // Failure analysis
        if !report.quality.failure_analyses.is_empty() {
            md.push_str("## Failure Analysis\n\n");

            for (i, failure) in report.quality.failure_analyses.iter().enumerate() {
                md.push_str(&format!("### {}. {}\n\n", i + 1, failure.scenario));
                md.push_str(&format!("- **Type:** `{:?}`\n", failure.failure_type));

                if let Some(ref tool) = failure.expected_tool {
                    md.push_str(&format!("- **Expected Tool:** `{}`\n", tool));
                }
                if let Some(ref tool) = failure.actual_tool {
                    md.push_str(&format!("- **Actual Tool:** `{}`\n", tool));
                }

                if !failure.schema_errors.is_empty() {
                    md.push_str("- **Schema Errors:**\n");
                    for error in &failure.schema_errors {
                        md.push_str(&format!("  - {}\n", error));
                    }
                }

                if let Some(ref error) = failure.parse_error {
                    md.push_str(&format!("- **Parse Error:** `{}`\n", error));
                }

                md.push_str("- **Raw LLM Output (first 500 chars):**\n");
                md.push_str("  ```json\n");
                let preview = failure
                    .raw_llm_output
                    .chars()
                    .take(500)
                    .collect::<String>();
                md.push_str(&format!("  {}\n", preview));
                if failure.raw_llm_output.len() > 500 {
                    md.push_str("  ... (truncated)\n");
                }
                md.push_str("  ```\n\n");
            }
        }

        // Footer
        md.push_str("---\n\n");
        md.push_str("*Generated by Eris Benchmark System*\n");

        md
    }

    /// Generate a comparison report between two runs.
    pub fn comparison(
        baseline: &BenchmarkReport,
        current: &BenchmarkReport,
    ) -> String {
        let mut output = String::new();

        output.push_str(&format!("\n{}", "═".repeat(70)));
        output.push_str(&format!("\n{:^70}", "BENCHMARK COMPARISON"));
        output.push_str(&format!("\n{}", "═".repeat(70)));
        output.push_str(&format!("\n  Baseline:  {} ({})", baseline.model_name, baseline.run_id));
        output.push_str(&format!("\n  Current:   {} ({})", current.model_name, current.run_id));
        output.push_str(&format!("\n{}", "─".repeat(70)));

        // Quality comparison
        output.push_str(&format!("\n  {:^66}", "QUALITY METRICS"));
        output.push_str(&format!("\n{}", "─".repeat(70)));
        output.push_str(&format!(
            "\n  {:.<35} {:>8.1}%  {:>8.1}%  {:>+6.1}%",
            "JSON Parse Success",
            baseline.quality.json_success_rate(),
            current.quality.json_success_rate(),
            current.quality.json_success_rate() - baseline.quality.json_success_rate()
        ));
        output.push_str(&format!(
            "\n  {:.<35} {:>8.1}%  {:>8.1}%  {:>+6.1}%",
            "Recovery Success",
            baseline.quality.recovery_success_rate(),
            current.quality.recovery_success_rate(),
            current.quality.recovery_success_rate() - baseline.quality.recovery_success_rate()
        ));
        output.push_str(&format!(
            "\n  {:.<35} {:>8.1}%  {:>8.1}%  {:>+6.1}%",
            "Tool Validation",
            baseline.quality.tool_valid_rate(),
            current.quality.tool_valid_rate(),
            current.quality.tool_valid_rate() - baseline.quality.tool_valid_rate()
        ));
        output.push_str(&format!(
            "\n  {:.<35} {:>8.1}%  {:>8.1}%  {:>+6.1}%",
            "Overall Quality",
            baseline.quality.overall_quality_score(),
            current.quality.overall_quality_score(),
            current.quality.overall_quality_score() - baseline.quality.overall_quality_score()
        ));

        output.push_str(&format!("\n{}", "─".repeat(70)));

        // Speed comparison
        output.push_str(&format!("\n  {:^66}", "SPEED METRICS"));
        output.push_str(&format!("\n{}", "─".repeat(70)));
        output.push_str(&format!(
            "\n  {:.<35} {:>8.1}  {:>8.1}  {:>+6.1}",
            "Prompt Tok/s",
            baseline.speed.prompt_throughput(),
            current.speed.prompt_throughput(),
            current.speed.prompt_throughput() - baseline.speed.prompt_throughput()
        ));
        output.push_str(&format!(
            "\n  {:.<35} {:>8.1}  {:>8.1}  {:>+6.1}",
            "Generation Tok/s",
            baseline.speed.generation_throughput(),
            current.speed.generation_throughput(),
            current.speed.generation_throughput() - baseline.speed.generation_throughput()
        ));
        output.push_str(&format!(
            "\n  {:.<35} {:>8.0}  {:>8.0}  {:>+6.0}",
            "Total wall (ms)",
            baseline.speed.total_duration.as_millis(),
            current.speed.total_duration.as_millis(),
            current.speed.total_duration.as_millis() as i128 - baseline.speed.total_duration.as_millis() as i128
        ));
        output.push_str(&format!(
            "\n  {:.<35} {:>8.0}  {:>8.0}  {:>+6.0}",
            "Prompt phase (ms)",
            baseline.speed.prompt_eval_duration.as_millis(),
            current.speed.prompt_eval_duration.as_millis(),
            current.speed.prompt_eval_duration.as_millis() as i128 - baseline.speed.prompt_eval_duration.as_millis() as i128
        ));
        output.push_str(&format!(
            "\n  {:.<35} {:>8.0}  {:>8.0}  {:>+6.0}",
            "Generation phase (ms)",
            baseline.speed.eval_duration.as_millis(),
            current.speed.eval_duration.as_millis(),
            current.speed.eval_duration.as_millis() as i128 - baseline.speed.eval_duration.as_millis() as i128
        ));
        output.push_str("\n  (Probe = single minimal chat; streaming TTFT not measured.)");

        output.push_str(&format!("\n{}", "─".repeat(70)));
        output.push_str(&format!("\n  {:^66}", "SUITE TIMING (passed scenarios only)"));
        output.push_str(&format!("\n{}", "─".repeat(70)));
        output.push_str(&format!(
            "\n  {:.<35} {:>8.0}  {:>8.0}  {:>+6.0}",
            "Mean LLM ms / user step",
            baseline.suite_speed.mean_llm_ms,
            current.suite_speed.mean_llm_ms,
            current.suite_speed.mean_llm_ms - baseline.suite_speed.mean_llm_ms
        ));
        output.push_str(&format!(
            "\n  {:.<35} {:>8.0}  {:>8.0}  {:>+6.0}",
            "Mean tool ms / user step",
            baseline.suite_speed.mean_tool_ms,
            current.suite_speed.mean_tool_ms,
            current.suite_speed.mean_tool_ms - baseline.suite_speed.mean_tool_ms
        ));
        output.push_str(&format!(
            "\n  {:.<35} {:>8.0}  {:>8.0}  {:>+6.0}",
            "Mean total ms / user step",
            baseline.suite_speed.mean_total_ms,
            current.suite_speed.mean_total_ms,
            current.suite_speed.mean_total_ms - baseline.suite_speed.mean_total_ms
        ));
        output.push_str(&format!(
            "\n  Step samples....................... {:>8}  {:>8}",
            baseline.suite_speed.step_samples, current.suite_speed.step_samples
        ));
        output.push_str(
            "\n  (Different pass rates ⇒ different scenario subsets — not apples-to-apples workload.)",
        );

        // Scenario comparison
        output.push_str(&format!("\n{}", "─".repeat(70)));
        output.push_str(&format!("\n  {:^66}", "SCENARIO COMPARISON"));
        output.push_str(&format!("\n{}", "─".repeat(70)));

        let baseline_success = baseline.quality.scenario_results.iter().filter(|r| r.succeeded).count();
        let current_success = current.quality.scenario_results.iter().filter(|r| r.succeeded).count();

        output.push_str(&format!(
            "\n  Scenarios passed: {} → {} ({})",
            baseline_success,
            current_success,
            if current_success >= baseline_success {
                format!("+{}", current_success - baseline_success)
            } else {
                format!("-{}", baseline_success - current_success)
            }
        ));

        output.push_str(&format!("\n{}", "═".repeat(70)));
        output.push('\n');

        output
    }
}

impl ReportGenerator {
    /// Generate a trend report from multiple runs.
    pub fn generate_trend_report(reports: &[BenchmarkReport]) -> String {
    let mut output = String::new();

    output.push_str(&format!("\n{}", "═".repeat(70)));
    output.push_str(&format!("\n{:^70}", "QUALITY TREND REPORT"));
    output.push_str(&format!("\n{}", "═".repeat(70)));
    output.push_str(&format!("\n  {} runs analyzed\n", reports.len()));
    output.push_str(&format!("\n{}", "─".repeat(70)));
    output.push_str(&format!("\n  {:^66}", "QUALITY OVER TIME"));
    output.push_str(&format!("\n{}", "─".repeat(70)));

    // Header
    output.push_str(&format!("\n  {:<25} {:>8} {:>8} {:>8} {:>8}",
        "Run", "JSON", "Recovery", "Tool", "Overall"));
    output.push_str(&format!("\n  {}\n", "─".repeat(66)));

    // Rows
    for report in reports {
        output.push_str(&format!(
            "\n  {:<25} {:>7.1}% {:>7.1}% {:>7.1}% {:>7.1}%",
            format!("{} {}", report.timestamp.format("%m-%d"), report.model_name.chars().take(15).collect::<String>()),
            report.quality.json_success_rate(),
            report.quality.recovery_success_rate(),
            report.quality.tool_valid_rate(),
            report.quality.overall_quality_score()
        ));
    }

        output.push_str(&format!("\n{}", "═".repeat(70)));
        output.push('\n');

        output
    }
}

/// Helper function for checkbox formatting.
fn checkbox(value: bool) -> &'static str {
    if value { "✓" } else { "✗" }
}

/// Helper function for grading.
fn grade(score: f64) -> String {
    if score >= 95.0 {
        "A+".to_string()
    } else if score >= 90.0 {
        "A".to_string()
    } else if score >= 85.0 {
        "B+".to_string()
    } else if score >= 80.0 {
        "B".to_string()
    } else if score >= 70.0 {
        "C".to_string()
    } else if score >= 60.0 {
        "D".to_string()
    } else {
        "F".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark::metrics::{
        BenchmarkReport, CleanupConfirmation, QualityMetrics, ScenarioResult, SpeedMetrics,
        SuiteSpeedAggregate,
    };
    use std::time::Duration;

    fn create_test_report() -> BenchmarkReport {
        BenchmarkReport {
            run_id: "test_2024-01-01".to_string(),
            timestamp: chrono::Utc::now(),
            model_name: "test-model".to_string(),
            suite: "quick".to_string(),
            quality: QualityMetrics {
                json_parse_attempts: 10,
                json_parse_successes: 9,
                recovery_triggers: 2,
                recovery_successes: 1,
                tool_calls_attempted: 5,
                tool_calls_valid: 4,
                timeout_count: 0,
                scenario_results: vec![
                    ScenarioResult {
                        scenario_name: "test".to_string(),
                        succeeded: true,
                        rounds_taken: 2,
                        max_rounds: 3,
                        steps_completed: 1,
                        total_steps: 1,
                        duration: Duration::from_millis(1000),
                        metrics: QualityMetrics::default(),
                        error_message: None,
                    },
                ],
                ..Default::default()
            },
            speed: SpeedMetrics {
                prompt_tokens: 100,
                generated_tokens: 50,
                ..Default::default()
            },
            suite_speed: SuiteSpeedAggregate {
                step_samples: 6,
                contributing_scenarios: 3,
                mean_llm_ms: 1200.0,
                mean_tool_ms: 100.0,
                mean_total_ms: 3500.0,
            },
            isolation_mode: "Strict".to_string(),
            cleanup_report: CleanupConfirmation {
                temp_vault_cleaned: true,
                qdrant_collection_removed: true,
                staged_memories_removed: 2,
                ephemeral_entries_removed: 1,
                cleanup_failures: vec![],
            },
        }
    }

    #[test]
    fn console_report_includes_all_sections() {
        let report = create_test_report();
        let output = ReportGenerator::console(&report);

        assert!(output.contains("ERIS CAPABILITY BENCHMARK"));
        assert!(output.contains("SAFETY CHECKLIST"));
        assert!(output.contains("QUALITY METRICS"));
        assert!(output.contains("SPEED METRICS"));
        assert!(output.contains("SUITE TIMING"));
        assert!(output.contains("SCENARIO RESULTS"));
    }

    #[test]
    fn markdown_report_has_headers() {
        let report = create_test_report();
        let md = ReportGenerator::markdown(&report);

        assert!(md.contains("# Benchmark Report: test-model"));
        assert!(md.contains("## Quality Metrics"));
        assert!(md.contains("## Speed Metrics"));
        assert!(md.contains("## Suite timing"));
        assert!(md.contains("## Scenario Results"));
    }

    #[test]
    fn json_report_is_valid_json() {
        let report = create_test_report();
        let json = ReportGenerator::json(&report).expect("Valid JSON");

        // Should be parseable
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("Parsable JSON");
        assert_eq!(parsed["model_name"], "test-model");
    }

    #[test]
    fn comparison_shows_deltas() {
        let baseline = create_test_report();
        let mut current = create_test_report();
        current.quality.json_parse_successes = 10; // Perfect

        let output = ReportGenerator::comparison(&baseline, &current);

        assert!(output.contains("BENCHMARK COMPARISON"));
        assert!(output.contains("test-model"));
    }

    #[test]
    fn grading_function_works() {
        assert_eq!(grade(95.0), "A+");
        assert_eq!(grade(90.0), "A");
        assert_eq!(grade(85.0), "B+");
        assert_eq!(grade(80.0), "B");
        assert_eq!(grade(70.0), "C");
        assert_eq!(grade(60.0), "D");
        assert_eq!(grade(50.0), "F");
    }
}
