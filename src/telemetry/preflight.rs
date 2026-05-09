use crate::config::AppConfig;
use crate::executive::cli::Commands;
use crate::executive::error::{FcpError, Result};
use crate::executive::peripherals::{ollama_reachable, qdrant_reachable};

pub async fn run_preflight_checks(command: &Commands, config: &AppConfig) -> Result<()> {
    // Chat and Benchmark manage their own peripheral lifecycle
    if matches!(command, Commands::Chat { .. } | Commands::Benchmark { .. }) {
        return Ok(());
    }

    if !ollama_reachable(&config.ollama_host).await {
        return Err(FcpError::NetworkFault(
            "FATAL: Ollama daemon not responding. Ensure Ollama is running.".into(),
        ));
    }

    if !qdrant_reachable(&config.qdrant_url).await {
        return Err(FcpError::NetworkFault(
            "FATAL: Qdrant sidecar not detected. Run your vector db.".into(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn preflight_skips_chat_mode() {
        let mut config = AppConfig::default();
        config.ollama_host = "not a url".into();
        config.qdrant_url = "still-not-a-url".into();
        let result = run_preflight_checks(&Commands::Chat { web: false }, &config).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn preflight_checks_non_chat_mode() {
        let mut config = AppConfig::default();
        config.ollama_host = "http://127.0.0.1:9".into();
        let result = run_preflight_checks(&Commands::Run { prompt: "x".into() }, &config).await;
        assert!(result.is_err());
    }
}
