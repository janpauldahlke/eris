use std::path::Path;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling;
use tracing_subscriber::{EnvFilter, fmt};

use crate::executive::error::Result;

/// Precedence: `RUST_LOG` env (highest) > CLI `-V` (forces debug) > `log_level` from config.
pub fn init_tracing(workspace_root: &Path, log_level: &str, verbose: bool) -> Result<WorkerGuard> {
    let log_dir = crate::vault_layout::telemetry_logs_dir(workspace_root);

    // Explicitly create the directory before initializing the appender
    std::fs::create_dir_all(&log_dir).map_err(crate::executive::error::FcpError::Io)?;

    let file_appender = rolling::daily(log_dir, "fcp_core.log");
    let (non_blocking_writer, guard) = tracing_appender::non_blocking(file_appender);

    let level = if verbose { "debug" } else { log_level };
    let configured = format!("eris={level},fcp={level}");
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(&configured))
        .unwrap_or_else(|_| EnvFilter::new("eris=info,fcp=info"));

    // `with_target(false)`: target strings are not printed. Orchestrator routing events use
    // `category = "routing"` (see `telemetry::routing_codes`) so operators can grep log files.
    let subscriber = fmt::Subscriber::builder()
        .with_env_filter(filter)
        .with_thread_ids(true)
        .with_target(false)
        .with_writer(non_blocking_writer);

    // It's safe to ignore `SetGlobalDefaultError` if tracing is already initialized
    let _ = subscriber.try_init();

    Ok(guard)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_tracing_initialization_file_creates_dir() {
        let dir = tempdir().unwrap();
        let result = init_tracing(&dir.path().to_path_buf(), "info", false);
        assert!(result.is_ok());

        let log_dir = crate::vault_layout::telemetry_logs_dir(dir.path());
        assert!(log_dir.exists());
    }
}
