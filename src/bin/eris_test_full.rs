//! Runs the full unit suite in subprocess batches so RSS resets between modules.
//! Single-process `cargo test` can OOM on laptops even with `--test-threads=1`.
//!
//! Progress is appended to `target/test-full.log` — after a session drop:
//!   tail -20 target/test-full.log

#![forbid(unsafe_code)]

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

/// Substring filters passed to `cargo test --bin eris <filter>`.
const BATCHES: &[&str] = &[
    "config::",
    "engine::grammar::",
    "engine::router::",
    "engine::token_metrics::",
    "engine::embedding::",
    "engine::ollama::",
    "engine::llama_cpp::",
    "memory::",
    "executive::",
    "benchmark::",
    "presentation::",
    "telemetry::",
    "ui::",
    "skills::",
    "workspace::",
    "ingest::",
    "util::",
    "orchestrator::core::",
    "orchestrator::context::",
    "orchestrator::llm_support::",
    "orchestrator::tool_router::",
    "orchestrator::state::",
    "orchestrator::r#loop::",
    "orchestrator::heartbeat::",
    "orchestrator::alarms::",
    "tools::web::",
    "tools::vault::",
    "tools::agenda::",
    "tools::mail::",
    "tools::gatekeeper::",
    "tools::moltbook::",
    "tools::memory::",
    "tools::db_rest::",
    "tools::validation::",
    "tools::skills::",
    "tools::weather::",
    "tools::wiki::",
    "tools::system::",
    "tools::vision::",
    "tools::clock::",
    "tools::calendar::",
];

const LOG_FILE: &str = "target/test-full.log";

fn log_path() -> PathBuf {
    PathBuf::from(LOG_FILE)
}

fn append_log(line: &str) {
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(log_path()) {
        let _ = writeln!(f, "{line}");
    }
}

fn banner(index: usize, total: usize, filter: &str, phase: &str) {
    let sep = "=".repeat(62);
    let block = format!("\n{sep}\n  [{index}/{total}] {phase}: {filter}\n{sep}\n");
    let _ = writeln!(io::stderr(), "{block}");
    append_log(&format!("[{index}/{total}] {phase}: {filter}"));
}

fn warm_test_binary() -> Result<(), String> {
    let _ = writeln!(io::stderr(), "eris test-full: building test binary (once)...");
    append_log("=== cargo build --bin eris --tests ===");
    let status = Command::new("cargo")
        .args(["build", "--bin", "eris", "--tests"])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("failed to spawn cargo build: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("cargo build --bin eris --tests failed".into())
    }
}

fn run_batch(index: usize, total: usize, filter: &str) -> Result<bool, String> {
    banner(index, total, filter, "START");
    let status = Command::new("cargo")
        .args([
            "test",
            "--bin",
            "eris",
            filter,
            "--",
            "--test-threads=1",
        ])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("failed to spawn cargo test for {filter}: {e}"))?;
    let ok = status.success();
    let phase = if ok { "DONE ok" } else { "DONE FAILED" };
    banner(index, total, filter, phase);
    Ok(ok)
}

fn main() -> ExitCode {
    let total = BATCHES.len();
    let _ = writeln!(
        io::stderr(),
        "eris test-full: {total} batches — log: {LOG_FILE}\n"
    );
    append_log(&format!("=== eris test-full run ({total} batches) ==="));

    if let Err(e) = warm_test_binary() {
        let _ = writeln!(io::stderr(), "{e}");
        append_log(&format!("*** ERROR: {e} ***"));
        return ExitCode::from(1);
    }

    let mut failed = false;
    for (i, filter) in BATCHES.iter().enumerate() {
        let index = i + 1;
        match run_batch(index, total, filter) {
            Ok(true) => {}
            Ok(false) => {
                let _ = writeln!(
                    io::stderr(),
                    "\n*** batch [{index}/{total}] failed: {filter} ***\n"
                );
                append_log(&format!("*** FAILED: {filter} ***"));
                failed = true;
                break;
            }
            Err(e) => {
                let _ = writeln!(io::stderr(), "{e}");
                append_log(&format!("*** ERROR: {e} ***"));
                return ExitCode::from(1);
            }
        }
    }
    if failed {
        let _ = writeln!(io::stderr(), "see tail -20 {LOG_FILE}");
        ExitCode::from(1)
    } else {
        let _ = writeln!(io::stderr(), "=== eris test-full: all {total} batches passed ===");
        append_log("=== all batches passed ===");
        ExitCode::SUCCESS
    }
}
