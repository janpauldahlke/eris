//! Benchmark runner - main entry point for executing benchmarks.

use crate::benchmark::{
    BenchmarkHarness, BenchmarkReport, IsolationMode, QualityMetrics, SpeedMetrics, SuiteRegistry,
};
use crate::benchmark::metrics::{StepTiming, SuiteSpeedAggregate};
use crate::config::AppConfig;
use crate::engine::ollama::OllamaClient;
use crate::engine::token_metrics;
use crate::executive::error::{FcpError, Result};
use crate::executive::peripherals::ensure_peripherals_for_chat;
use crate::memory::ephemeral::EphemeralMemory;
use crate::orchestrator::core::Orchestrator;
use crate::tools::Gatekeeper;
use ollama_rs::Ollama;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing;

/// Run a benchmark suite with the given configuration.
pub async fn run_benchmark(
    config: &AppConfig,
    suite_name: &str,
    output_format: &str,
    _compare: bool,
    output_path: Option<PathBuf>,
    isolation_mode: IsolationMode,
) -> Result<BenchmarkReport> {
    tracing::info!(
        suite = suite_name,
        model = %config.model_name,
        isolation = ?isolation_mode,
        "Starting benchmark run"
    );

    // Get the scenario suite
    let registry = SuiteRegistry::new();
    let suite = registry
        .get(suite_name)
        .ok_or_else(|| FcpError::Config(
            format!("Unknown benchmark suite: {}", suite_name)
        ))?;

    tracing::info!(
        suite = suite_name,
        scenarios = suite.len(),
        "Loaded scenario suite"
    );

    // Start peripherals (Ollama + Qdrant if available)
    println!("[benchmark] Checking peripheral daemons (Ollama, Qdrant)...");
    let peripheral_lifecycle = ensure_peripherals_for_chat(config).await?;
    
    let ollama_status = if peripheral_lifecycle.started_ollama() {
        "started by eris"
    } else {
        "already running"
    };
    let qdrant_status = if peripheral_lifecycle.started_qdrant() {
        "started by eris"
    } else {
        "already running"
    };
    println!("[benchmark] Peripheral readiness: ollama={ollama_status}, qdrant={qdrant_status}");

    // Set up cancellation token for benchmark
    let _cancel_token = CancellationToken::new();

    // Create Ollama client
    let parsed_url = url::Url::parse(&config.ollama_host)
        .map_err(|e| FcpError::Config(format!("Invalid ollama_host URL: {}", e)))?;
    let host = format!(
        "{}://{}",
        parsed_url.scheme(),
        parsed_url.host_str().unwrap_or("localhost")
    );
    let port = parsed_url.port().unwrap_or(11434);

    let client = Ollama::new(host, port);
    let (token_metrics_tx, _token_metrics_rx) = token_metrics::channel();
    let engine = OllamaClient::with_token_metrics(client.clone(), Arc::new(config.clone()), token_metrics_tx);
    let engine_arc = Arc::new(engine);

    println!("[benchmark] Sampling model latency (one chat probe; Ollama-reported tokens & timings)...");
    let speed_sample =
        match crate::benchmark::speed_probe::probe_ollama_chat_latency(engine_arc.as_ref()).await {
            Ok(s) => {
                tracing::info!(
                    prompt_tok_s = s.prompt_throughput(),
                    gen_tok_s = s.generation_throughput(),
                    total_ms = s.total_duration.as_millis(),
                    "Benchmark speed probe completed"
                );
                s
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Benchmark speed probe failed; speed metrics will be zeros"
                );
                SpeedMetrics::default()
            }
        };

    // Create a second engine for the orchestrator
    let (token_metrics_tx2, _token_metrics_rx2) = token_metrics::channel();
    let engine_for_orchestrator = OllamaClient::with_token_metrics(client.clone(), Arc::new(config.clone()), token_metrics_tx2);

    // Set up ephemeral memory
    let ephemeral = Arc::new(EphemeralMemory::new(config.workspace.clone()));

    // Set up semantic brain (Qdrant) if available
    let ollama_arc = Arc::new(client);
    let config_arc = Arc::new(config.clone());
    let semantic_arc: Option<Arc<crate::memory::semantic::SemanticBrain>> = 
        match crate::memory::semantic::SemanticBrain::new_with_connect_retries(
            config_arc.clone(),
            ollama_arc,
            config_arc.semantic_brain_connect_attempts,
            config_arc.semantic_brain_connect_retry_delay_ms,
        )
        .await
        {
            Ok(semantic_brain) => {
                let semantic = Arc::new(semantic_brain);
                tracing::info!("Benchmark: Semantic Brain online. Vector tools registered.");
                Some(semantic)
            }
            Err(e) => {
                if config.require_semantic_brain {
                    return Err(FcpError::VectorDbOffline(format!(
                        "Benchmark requires semantic brain but Qdrant is unreachable: {e}"
                    )));
                }
                tracing::warn!(
                    error = %e,
                    "Benchmark: Semantic Brain offline. Vector tools will be unavailable."
                );
                None
            }
        };

    // Build tool gatekeeper with only safe tools for benchmarks
    let mut gatekeeper = Gatekeeper::new();
    let read_limit = (config.num_ctx as f32 * config.vault_read_ratio) as usize;
    let workspace_root = config.active_vault();

    // Core safe tools (no external side effects)
    gatekeeper.register(Arc::new(crate::tools::vault::VaultReadTool {
        workspace_root: workspace_root.clone(),
        read_limit,
    }));
    gatekeeper.register(Arc::new(crate::tools::vault::VaultListTool {
        workspace_root: workspace_root.clone(),
    }));
    gatekeeper.register(Arc::new(crate::tools::vault::VaultSearchTool {
        workspace_root: workspace_root.clone(),
        max_files: config.vault_search_max_files,
        max_snippets_per_file: config.vault_search_max_snippets_per_file,
        snippet_radius_lines: config.vault_search_snippet_radius_lines,
        max_total_chars: config.vault_search_max_total_chars,
        max_file_bytes: config.vault_search_max_file_bytes,
    }));
    gatekeeper.register(Arc::new(crate::tools::system::SystemHealthTool {
        config: config_arc.clone(),
    }));
    gatekeeper.register(Arc::new(crate::tools::clock::ClockNowTool));
    
    // Memory tools (operate on ephemeral only, safe)
    if let Some(ref semantic) = semantic_arc {
        gatekeeper.register(Arc::new(crate::tools::memory::MemoryStageTool {
            config: config_arc.clone(),
            ephemeral: ephemeral.clone(),
            max_content_chars: config.num_ctx * 3,
        }));
        gatekeeper.register(Arc::new(crate::tools::memory::MemoryStagedListTool {
            ephemeral: ephemeral.clone(),
        }));
        gatekeeper.register(Arc::new(crate::tools::memory::MemoryCommitTool {
            workspace_root: workspace_root.clone(),
            semantic: semantic.clone(),
            ephemeral: ephemeral.clone(),
        }));
        gatekeeper.register(Arc::new(crate::tools::memory::MemoryCommitAllTool {
            workspace_root: workspace_root.clone(),
            semantic: semantic.clone(),
            ephemeral: ephemeral.clone(),
        }));
        gatekeeper.register(Arc::new(crate::tools::memory::MemoryQueryTool {
            workspace: config.workspace.clone(),
            semantic: semantic.clone(),
            default_top_k: config.memory_query_default_top_k,
            top_k_max: config.memory_query_top_k_max,
            default_max_total_chars: config.memory_query_default_max_total_chars,
            min_max_total_chars: config.memory_query_min_max_total_chars,
            qdrant_oversample_cap: config.memory_query_oversample_cap,
            qdrant_oversample_multiplier: config.memory_query_oversample_multiplier,
            qdrant_oversample_min: config.memory_query_oversample_min,
        }));
    }

    // Create orchestrator with correct signature
    let identity = Arc::from("Benchmark Identity");
    let (identity_tx, identity_rx) = tokio::sync::watch::channel(identity);
    drop(identity_tx);

    // Create interrupt receiver (not used in benchmarks but needed for API)
    let (_interrupt_tx, interrupt_rx) = tokio::sync::watch::channel(());

    let context_view = crate::orchestrator::context::ContextViewSettings::default();

    let mut orchestrator = Orchestrator::new(
        engine_for_orchestrator,
        gatekeeper,
        ephemeral.clone(),
        &config.active_vault(),
        &config.workspace,
        config.max_recovery_attempts,
        config.max_tool_rounds,
        config.condensation_threshold,
        config.num_ctx,
        config.tool_descriptor_jit_top_k,
        config.tool_descriptor_jit_max_chars,
        config.slim_tool_prompt,
        config.tool_map_offer_cap,
        interrupt_rx,
        None, // presentation_tx
        None, // tool_router
        None, // descriptor_registry
        context_view,
        config_arc.clone(),
        identity_rx,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    );

    let harness = BenchmarkHarness::new(&config.active_vault(), isolation_mode)?;

    // Run all scenarios (real orchestrator + LLM per step)
    println!("[benchmark] Running {} scenarios...", suite.len());
    let mut scenario_results = Vec::new();
    let mut suite_timing_steps: Vec<StepTiming> = Vec::new();
    let mut suite_timing_scenarios: u32 = 0;

    for (idx, scenario) in suite.scenarios.iter().enumerate() {
        println!("[benchmark] Scenario {}/{}: {}", idx + 1, suite.len(), scenario.name);

        match harness
            .run_scenario_with_orchestrator(
                &mut orchestrator,
                scenario,
                config.benchmark_scenario_timeout_secs,
            )
            .await
        {
            Ok((result, step_timings)) => {
                let status = if result.succeeded { "✓ PASS" } else { "✗ FAIL" };
                println!("[benchmark]   {} ({}ms)", status, result.duration.as_millis());
                if result.succeeded {
                    suite_timing_scenarios = suite_timing_scenarios.saturating_add(1);
                    suite_timing_steps.extend(step_timings);
                }
                scenario_results.push(result);
            }
            Err(e) => {
                println!("[benchmark]   ✗ ERROR: {}", e);
                tracing::error!(
                    scenario = %scenario.name,
                    error = %e,
                    "Scenario failed"
                );
                // Create a failure result
                scenario_results.push(crate::benchmark::ScenarioResult {
                    scenario_name: scenario.name.clone(),
                    succeeded: false,
                    rounds_taken: 0,
                    max_rounds: 0,
                    steps_completed: 0,
                    total_steps: scenario.steps.len() as u32,
                    duration: std::time::Duration::from_secs(0),
                    metrics: QualityMetrics::default(),
                    error_message: Some(e.to_string()),
                });
            }
        }
    }

    // Collect metrics and attach per-scenario results for reporting
    let mut quality_metrics = harness.metrics().await;
    for result in &scenario_results {
        quality_metrics.add_scenario_result(result.clone());
    }
    
    // Cleanup
    println!("[benchmark] Cleaning up...");
    let cleanup_report = harness.cleanup().await?;

    // Shutdown peripherals that were started by this benchmark
    let eris_owned_ollama = peripheral_lifecycle.started_ollama();
    let mut lifecycle = peripheral_lifecycle;
    let stopped = lifecycle.shutdown_async().await;
    if !stopped.is_empty() {
        tracing::info!(stopped = ?stopped, "Benchmark stopped managed peripherals");
    }

    // Match chat exit: free VRAM on a long-lived host Ollama (`ollama stop` per model).
    if config.unload_ollama_models_on_chat_exit && !eris_owned_ollama {
        tracing::info!(
            chat_model = %config.model_name,
            embed_model = %config.embed_model_name,
            "Unloading benchmark models via `ollama stop` (host Ollama left running)"
        );
        crate::executive::peripherals::unload_ollama_models_cli_best_effort(config).await;
    } else if config.unload_ollama_models_on_chat_exit && eris_owned_ollama {
        tracing::debug!(
            "Skipping `ollama stop` after benchmark; managed Ollama process for this run was stopped"
        );
    }

    let suite_speed =
        SuiteSpeedAggregate::from_step_samples(&suite_timing_steps, suite_timing_scenarios);

    // Build report
    let report = BenchmarkReport {
        run_id: crate::benchmark::storage::sanitize_run_id_for_path(&format!(
            "{}_{}",
            chrono::Utc::now().format("%Y-%m-%d_%H-%M-%S"),
            config.model_name
        )),
        timestamp: chrono::Utc::now(),
        model_name: config.model_name.clone(),
        suite: suite_name.to_string(),
        quality: quality_metrics,
        speed: speed_sample,
        suite_speed,
        isolation_mode: format!("{:?}", isolation_mode),
        cleanup_report: crate::benchmark::CleanupConfirmation {
            temp_vault_cleaned: cleanup_report.will_auto_cleanup,
            qdrant_collection_removed: true,
            staged_memories_removed: cleanup_report.staged_removed,
            ephemeral_entries_removed: cleanup_report.ephemeral_removed,
            cleanup_failures: cleanup_report.failures.iter().map(|f| f.error.clone()).collect(),
        },
    };

    // Output results
    match output_format {
        "table" => {
            print_console_report(&report);
        }
        "json" => {
            let json = serde_json::to_string_pretty(&report)?;
            if let Some(path) = output_path {
                tokio::fs::write(&path, json).await?;
                println!("Report saved to: {}", path.display());
            } else {
                println!("{}", json);
            }
        }
        "markdown" => {
            let md = generate_markdown_report(&report);
            if let Some(path) = output_path {
                tokio::fs::write(&path, md).await?;
                println!("Report saved to: {}", path.display());
            } else {
                println!("{}", md);
            }
        }
        _ => {
            return Err(FcpError::Config(
                format!("Unknown output format: {}", output_format)
            ));
        }
    }

    tracing::info!(
        run_id = %report.run_id,
        quality_score = %report.quality.overall_quality_score(),
        "Benchmark complete"
    );

    Ok(report)
}

/// Print console table report.
fn print_console_report(report: &BenchmarkReport) {
    println!();
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║  ERIS CAPABILITY BENCHMARK                                       ║");
    println!("╠══════════════════════════════════════════════════════════════════╣");
    println!("║  Model: {:58} ║", report.model_name);
    println!("║  Suite: {:58} ║", report.suite);
    println!("║  Run ID: {:57} ║", report.run_id);
    println!("╠══════════════════════════════════════════════════════════════════╣");
    println!("║  SAFETY CHECKLIST                                                ║");
    println!("╠══════════════════════════════════════════════════════════════════╣");
    println!("║  {} External side effects: BLOCKED                               ║",
        if report.cleanup_report.all_cleaned() { "✓" } else { "✗" });
    println!("║  {} Temp vault: cleaned                                          ║",
        if report.cleanup_report.temp_vault_cleaned { "✓" } else { "✗" });
    println!("║  ✓ Staged memories: {} removed                                   ║",
        report.cleanup_report.staged_memories_removed);
    println!("╠══════════════════════════════════════════════════════════════════╣");
    println!("║  QUALITY METRICS                                                 ║");
    println!("╠══════════════════════════════════════════════════════════════════╣");
    println!("║  JSON Parse Success:      {:6.1}%                                ║",
        report.quality.json_success_rate());
    println!("║  Recovery Success:        {:6.1}%                                ║",
        report.quality.recovery_success_rate());
    println!("║  Tool Valid Rate:          {:6.1}%                                ║",
        report.quality.tool_valid_rate());
    println!("║  Timeout Rate:            {:6.1}%                                ║",
        report.quality.timeout_rate());
    println!("║  Overall Quality Score:   {:6.1}%                                ║",
        report.quality.overall_quality_score());
    println!("╠══════════════════════════════════════════════════════════════════╣");
    println!("║  SPEED (Ollama probe + real scenario LLM/tool work below)        ║");
    println!("╠══════════════════════════════════════════════════════════════════╣");
    println!("║  Prompt throughput:    {:6.1} tok/s                             ║",
        report.speed.prompt_throughput());
    println!("║  Generation throughput:{:6.1} tok/s                             ║",
        report.speed.generation_throughput());
    println!("║  Total request (wall):  {:6} ms                                   ║",
        report.speed.total_duration.as_millis());
    println!("║  Prompt eval phase:     {:6} ms                                   ║",
        report.speed.prompt_eval_duration.as_millis());
    println!("║  Generation phase:      {:6} ms                                   ║",
        report.speed.eval_duration.as_millis());
    println!("╠══════════════════════════════════════════════════════════════════╣");
    println!("║  SUITE TIMING (passed scenarios only; mean per user step)        ║");
    println!("╠══════════════════════════════════════════════════════════════════╣");
    if report.suite_speed.step_samples > 0 {
        println!(
            "║  Steps averaged:      {:3}  ({} scenarios)                       ║",
            report.suite_speed.step_samples, report.suite_speed.contributing_scenarios
        );
        println!(
            "║  Mean LLM ms/step:    {:6.0}                                       ║",
            report.suite_speed.mean_llm_ms
        );
        println!(
            "║  Mean tool ms/step:   {:6.0}                                       ║",
            report.suite_speed.mean_tool_ms
        );
        println!(
            "║  Mean total ms/step:  {:6.0}                                       ║",
            report.suite_speed.mean_total_ms
        );
    } else {
        println!("║  (no passed scenarios — no suite timing aggregate)               ║");
    }
    println!("╠══════════════════════════════════════════════════════════════════╣");
    println!("║  SCENARIO RESULTS                                                ║");
    println!("╠══════════════════════════════════════════════════════════════════╣");
    println!("║  Scenarios run:     {:3}                                         ║",
        report.quality.scenario_results.len());
    println!("║  Successful:        {:3}                                         ║",
        report.quality.scenario_results.iter().filter(|r| r.succeeded).count());
    println!("║  Failed:           {:3}                                         ║",
        report.quality.scenario_results.iter().filter(|r| !r.succeeded).count());
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!();
    println!("  Report saved to: .fcp/benchmarks/{}.json", report.run_id);
    println!();
    println!("  View with:  eris benchmark --list");
    println!("  Compare (this vault): eris benchmark --diff '<run-id>..<run-id>'");
    println!("  Compare (two files):  eris benchmark --diff-files BASE.json OTHER.json");
    println!();
}

/// Generate markdown report.
fn generate_markdown_report(report: &BenchmarkReport) -> String {
    let mut md = String::new();
    
    md.push_str(&format!("# Benchmark Report: {}\n\n", report.model_name));
    md.push_str("## Metadata\n\n");
    md.push_str("| Field | Value |\n");
    md.push_str("|-------|-------|\n");
    md.push_str(&format!("| **Run ID** | `{}` |\n", report.run_id));
    md.push_str(&format!("| **Suite** | {} |\n", report.suite));
    md.push_str(&format!("| **Date** | {} |\n", report.timestamp.format("%Y-%m-%d %H:%M:%S UTC")));
    md.push_str(&format!("| **Isolation Mode** | {} |\n\n", report.isolation_mode));

    md.push_str("## Safety Checklist\n\n");
    md.push_str("- [x] External side effects blocked\n");
    md.push_str(&format!(
        "- [{}] Temp vault cleaned\n",
        if report.cleanup_report.temp_vault_cleaned { "x" } else { " " }
    ));
    md.push_str(&format!(
        "- [x] {} staged memories removed\n\n",
        report.cleanup_report.staged_memories_removed
    ));

    md.push_str("## Quality Metrics\n\n");
    md.push_str("| Metric | Value |\n");
    md.push_str("|--------|-------|\n");
    md.push_str(&format!("| JSON Parse Success | {:.1}% |\n", report.quality.json_success_rate()));
    md.push_str(&format!("| Recovery Success | {:.1}% |\n", report.quality.recovery_success_rate()));
    md.push_str(&format!("| Tool Valid Rate | {:.1}% |\n", report.quality.tool_valid_rate()));
    md.push_str(&format!("| Timeout Rate | {:.1}% |\n", report.quality.timeout_rate()));
    md.push_str(&format!("| **Overall Quality Score** | **{:.1}%** |\n\n", 
        report.quality.overall_quality_score()));

    md.push_str("## Suite timing (passed scenarios)\n\n");
    md.push_str("Means over orchestrator `step()` completions from **successful scenarios only** (different pass rates ⇒ different workload mixes between models).\n\n");
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

    md.push_str("## Scenario Results\n\n");
    md.push_str("| Scenario | Status | Rounds | Duration |\n");
    md.push_str("|----------|--------|--------|----------|\n");
    for result in &report.quality.scenario_results {
        let status = if result.succeeded { "✓ Pass" } else { "✗ Fail" };
        md.push_str(&format!("| {} | {} | {}/{} | {}ms |\n",
            result.scenario_name,
            status,
            result.rounds_taken,
            result.max_rounds,
            result.duration.as_millis()));
    }

    md.push('\n');
    md
}
