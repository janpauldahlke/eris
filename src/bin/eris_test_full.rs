//! Runs the full unit suite in subprocess batches — a **local mirror of the CI matrix**
//! in `.github/workflows/ci.yml` (47 filters). Codecov uses the same shards in CI;
//! you do not need this binary for coverage uploads.
//!
//! # When to use what
//!
//! - **`cargo test`** — daily dev (one process, all tests, fast).
//! - **`cargo test-full`** — optional pre-push parity with CI batch list.
//! - **`./scripts/test-full-detached.sh`** — full suite without GNOME session roulette.
//!
//! # Issue: `hybrid-gpu-gnome-session-drop` (Heisenbug)
//!
//! On some hybrid AMD iGPU + NVIDIA desktops, **long interactive** `test-full` from
//! Cursor/GNOME can log the user out (`exit.target`, amdgpu LTTPR in journal) even
//! when tests are fine. Quiet mode and batch splits help but do not fix the desktop.
//! See `docs/TODO/SOFTEN_TEST_FULL_OOM.md`.
//!
//! Default alias is **quiet** (batch output → `target/test-full.log` only).
//!
//! # Resume after a session drop
//!
//! ```bash
//! ./scripts/test-full-detached.sh   # preferred on FUCKUP-class hosts
//! ERIS_TEST_FROM=13 cargo test-full
//! tail -20 target/test-full.log
//! ```

#![forbid(unsafe_code)]

use std::env;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::thread;
use std::time::Duration;

/// Substring filters passed to the prebuilt test binary.
const BATCHES: &[&str] = &[
    "config::",
    "engine::grammar::",
    "engine::router::",
    "engine::token_metrics::",
    "engine::embedding::",
    "engine::ollama::",
    "engine::llama_cpp::",
    "memory::",
    "executive::cli::",
    "executive::error::",
    "executive::identity_md::",
    "executive::setup_welder::",
    "executive::peripherals::",
    "executive::router::",
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
    "tools::media::",
    "tools::clock::",
    "tools::calendar::",
];

const LOG_FILE: &str = "target/test-full.log";
const PROGRESS_FILE: &str = "target/test-full.progress";

fn log_path() -> PathBuf {
    PathBuf::from(LOG_FILE)
}

fn append_log(line: &str) {
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(log_path()) {
        let _ = writeln!(f, "{line}");
    }
}

fn write_progress(index: usize, total: usize, filter: &str, phase: &str) {
    let line = format!("[{index}/{total}] {phase}: {filter}");
    let _ = writeln!(io::stderr(), "{line}");
    append_log(&line);
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(PROGRESS_FILE)
    {
        let _ = writeln!(f, "{line}");
    }
}

fn quiet_mode() -> bool {
    if env::var("ERIS_TEST_QUIET")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    {
        return true;
    }
    env::args().any(|a| a == "--quiet" || a == "-q")
}

fn batch_pause() -> Duration {
    let secs = env::var("ERIS_TEST_PAUSE_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(if quiet_mode() { 3 } else { 1 });
    Duration::from_secs(secs)
}

fn parse_start_index() -> usize {
    let args: Vec<String> = env::args().collect();
    let mut from_cli: Option<usize> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--from" => {
                if let Some(raw) = args.get(i + 1) {
                    if let Ok(n) = raw.parse::<usize>() {
                        from_cli = Some(n);
                    }
                    i += 2;
                    continue;
                }
            }
            "--quiet" | "-q" => {
                i += 1;
                continue;
            }
            _ => {}
        }
        i += 1;
    }

    if let Some(n) = from_cli {
        return n;
    }
    if let Ok(raw) = env::var("ERIS_TEST_FROM") {
        if let Ok(n) = raw.parse::<usize>() {
            return n;
        }
    }
    resume_from_log().unwrap_or(1)
}

/// Scan `target/test-full.log` for the last `[N/M] DONE ok:` batch marker.
fn resume_from_log() -> Option<usize> {
    let content = std::fs::read_to_string(log_path()).ok()?;
    for line in content.lines().rev() {
        let line = line.trim();
        if !line.starts_with('[') || !line.contains("] DONE ok: ") {
            continue;
        }
        let rest = line.strip_prefix('[')?;
        let (num_and_total, rest) = rest.split_once(']')?;
        let filter = rest.strip_prefix(" DONE ok: ")?.trim();
        let batch_num = num_and_total.split_once('/')?.0.parse::<usize>().ok()?;
        if BATCHES.iter().any(|b| *b == filter) {
            return Some(batch_num + 1);
        }
    }
    None
}

fn open_log_stdio() -> Result<Stdio, String> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
        .map(Stdio::from)
        .map_err(|e| format!("failed to open {} for batch output: {e}", log_path().display()))
}

fn warm_test_binary(quiet: bool) -> Result<(), String> {
    let _ = writeln!(
        io::stderr(),
        "eris test-full: building test binary (once)..."
    );
    append_log("=== cargo build --bin eris --tests ===");
    let mut cmd = Command::new("cargo");
    cmd.args(["build", "--bin", "eris", "--tests"]);
    if quiet {
        let log_stdio = open_log_stdio()?;
        cmd.stdout(log_stdio).stderr(Stdio::from(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_path())
                .map_err(|e| format!("failed to open build log: {e}"))?,
        ));
    } else {
        cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    }
    let status = cmd
        .status()
        .map_err(|e| format!("failed to spawn cargo build: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("cargo build --bin eris --tests failed".into())
    }
}

fn is_test_executable(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if !name.starts_with("eris-") {
        return false;
    }
    if name.ends_with(".d") || name.ends_with(".rlib") || name.ends_with(".rmeta") {
        return false;
    }
    if name.contains(".long-type-") {
        return false;
    }
    path.is_file()
}

fn find_test_executable() -> Result<PathBuf, String> {
    let deps = PathBuf::from("target/debug/deps");
    let entries = std::fs::read_dir(&deps)
        .map_err(|e| format!("failed to read {}: {e}", deps.display()))?;

    let mut newest: Option<(PathBuf, std::time::SystemTime)> = None;
    for entry in entries {
        let entry = entry.map_err(|e| format!("failed to read deps entry: {e}"))?;
        let path = entry.path();
        if !is_test_executable(&path) {
            continue;
        }
        let modified = entry
            .metadata()
            .map_err(|e| format!("failed to stat {}: {e}", path.display()))?
            .modified()
            .map_err(|e| format!("failed to read mtime for {}: {e}", path.display()))?;
        match &newest {
            Some((_, best)) if modified <= *best => {}
            _ => newest = Some((path, modified)),
        }
    }

    newest
        .map(|(path, _)| path)
        .ok_or_else(|| format!("no test executable found in {}", deps.display()))
}

fn run_batch(
    index: usize,
    total: usize,
    filter: &str,
    test_exe: &Path,
    quiet: bool,
) -> Result<bool, String> {
    write_progress(index, total, filter, "START");
    if quiet {
        append_log(&format!("--- batch output: {filter} ---"));
    }

    let mut cmd = Command::new(test_exe);
    cmd.arg(filter)
        .arg("--test-threads=1")
        .env("MALLOC_ARENA_MAX", "2");

    if quiet {
        let out = open_log_stdio()?;
        let err = open_log_stdio()?;
        cmd.stdout(out).stderr(err);
    } else {
        cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    }

    let status = cmd
        .status()
        .map_err(|e| format!("failed to spawn test binary for {filter}: {e}"))?;
    let ok = status.success();
    let phase = if ok { "DONE ok" } else { "DONE FAILED" };
    write_progress(index, total, filter, phase);
    Ok(ok)
}

fn main() -> ExitCode {
    let quiet = quiet_mode();
    let total = BATCHES.len();
    let start = parse_start_index().clamp(1, total);
    let mode = if quiet { "quiet" } else { "verbose" };
    let _ = writeln!(
        io::stderr(),
        "eris test-full ({mode}): {total} batches — log: {LOG_FILE} — progress: {PROGRESS_FILE}\n"
    );
    append_log(&format!("=== eris test-full run ({total} batches, {mode}) ==="));
    if start > 1 {
        let msg = format!("=== resuming from batch {start} ===");
        let _ = writeln!(io::stderr(), "{msg}");
        append_log(&msg);
    }

    if let Err(e) = warm_test_binary(quiet) {
        let _ = writeln!(io::stderr(), "{e}");
        append_log(&format!("*** ERROR: {e} ***"));
        return ExitCode::from(1);
    }

    let test_exe = match find_test_executable() {
        Ok(path) => {
            let _ = writeln!(
                io::stderr(),
                "eris test-full: using test binary {}",
                path.display()
            );
            path
        }
        Err(e) => {
            let _ = writeln!(io::stderr(), "{e}");
            append_log(&format!("*** ERROR: {e} ***"));
            return ExitCode::from(1);
        }
    };

    let pause = batch_pause();
    let mut failed = false;
    for (i, filter) in BATCHES.iter().enumerate() {
        let index = i + 1;
        if index < start {
            continue;
        }
        if index > start {
            thread::sleep(pause);
        }
        match run_batch(index, total, filter, &test_exe, quiet) {
            Ok(true) => {}
            Ok(false) => {
                let _ = writeln!(
                    io::stderr(),
                    "\n*** batch [{index}/{total}] failed: {filter} — see {LOG_FILE} ***\n"
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
        let _ = writeln!(io::stderr(), "see tail -40 {LOG_FILE}");
        ExitCode::from(1)
    } else {
        let _ = writeln!(io::stderr(), "=== eris test-full: all {total} batches passed ===");
        append_log("=== all batches passed ===");
        ExitCode::SUCCESS
    }
}
