use std::path::Path;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling;
use tracing_subscriber::{fmt, EnvFilter};

use crate::executive::error::Result;

pub fn init_tracing(workspace_root: &Path) -> Result<WorkerGuard> {
    let log_dir = workspace_root.join(".fcp").join("logs");
    
    // Explicitly create the directory before initializing the appender
    std::fs::create_dir_all(&log_dir).map_err(crate::executive::error::FcpError::Io)?;

    let file_appender = rolling::daily(log_dir, "fcp_core.log");
    let (non_blocking_writer, guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("eris=debug,fcp=debug"));

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
        let result = init_tracing(&dir.path().to_path_buf());
        assert!(result.is_ok());
        
        let log_dir = dir.path().join(".fcp").join("logs");
        assert!(log_dir.exists());
    }
}
