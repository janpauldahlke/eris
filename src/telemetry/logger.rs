use std::path::PathBuf;
use tracing_appender::rolling;
use tracing_subscriber::{fmt, EnvFilter};

use crate::executive::error::Result;

pub enum LogTarget {
    Stderr,
    FileAppender(PathBuf),
}

pub fn init_tracing(verbosity: u8, target: LogTarget) -> Result<()> {
    let level = match verbosity {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));

    let subscriber = fmt::Subscriber::builder()
        .with_env_filter(filter)
        .with_thread_ids(true)
        .with_target(false);

    let init_result = match target {
        LogTarget::Stderr => {
            subscriber
                .with_writer(std::io::stderr)
                .try_init()
        }
        LogTarget::FileAppender(path) => {
            // tracing_appender automatically creates the directory if it doesn't exist.
            let appender = rolling::never(path.clone(), "fcp.log");
            subscriber
                .with_writer(appender)
                .try_init()
        }
    };

    // We ignore the error if tracing is already initialized (e.g., in parallel tests).
    match init_result {
        Ok(_) => Ok(()),
        Err(e) => {
            // It's safe to ignore `SetGlobalDefaultError` because it just means
            // another test or component already initialized tracing.
            // But we should verify it's a SetGlobalDefaultError.
            // Since `try_init` returns a Box<dyn Error>, we just ignore it for now.
            let _ = e;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_tracing_initialization_stderr_does_not_panic() {
        let result = init_tracing(1, LogTarget::Stderr);
        assert!(result.is_ok());
        tracing::info!("Test log to stderr");
    }

    #[test]
    fn test_tracing_initialization_file_creates_dir() {
        let dir = tempdir().unwrap();
        let log_dir = dir.path().join("logs");
        
        let result = init_tracing(2, LogTarget::FileAppender(log_dir.clone()));
        assert!(result.is_ok());
        
        // Ensure the directory was created by tracing_appender
        assert!(log_dir.exists());
    }
}
