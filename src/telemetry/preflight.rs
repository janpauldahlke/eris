use crate::config::{AppConfig, LlmBackend};
use crate::executive::cli::Commands;
use crate::executive::error::{FcpError, Result};
use crate::executive::peripherals::{llama_server_reachable, ollama_reachable, qdrant_grpc_ready};

pub async fn run_preflight_checks(command: &Commands, config: &AppConfig) -> Result<()> {
    // Chat and Benchmark manage their own peripheral lifecycle
    if matches!(command, Commands::Chat { .. } | Commands::Benchmark { .. }) {
        return Ok(());
    }

    match config.llm_backend {
        LlmBackend::Ollama => {
            if !ollama_reachable(&config.ollama_host).await {
                return Err(FcpError::NetworkFault(
                    "FATAL: Ollama daemon not responding. Ensure Ollama is running.".into(),
                ));
            }
        }
        LlmBackend::LlamaCpp => {
            if let Some(lc) = config.llama_cpp.as_ref() {
                if !llama_server_reachable(&lc.chat_server_url).await {
                    return Err(FcpError::NetworkFault(format!(
                        "FATAL: llama-server (chat) not responding at {}",
                        lc.chat_server_url
                    )));
                }
                if !llama_server_reachable(&lc.embed_server_url).await {
                    return Err(FcpError::NetworkFault(format!(
                        "FATAL: llama-server (embed) not responding at {}",
                        lc.embed_server_url
                    )));
                }
            }
        }
    }

    if !qdrant_grpc_ready(&config.qdrant_url).await {
        return Err(FcpError::NetworkFault(
            "FATAL: Qdrant is not answering gRPC at qdrant_url. Start Qdrant or fix the URL."
                .into(),
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

    #[tokio::test]
    async fn preflight_llamacpp_unreachable() {
        use crate::config::{LlamaCppConfig, LlmBackend};
        use std::path::PathBuf;

        let mut config = AppConfig::default();
        config.llm_backend = LlmBackend::LlamaCpp;
        config.llama_cpp = Some(LlamaCppConfig {
            home: PathBuf::from("/nonexistent"),
            chat_server_url: "http://127.0.0.1:9".into(),
            embed_server_url: "http://127.0.0.1:9".into(),
            chat_model_path: PathBuf::from("/x.gguf"),
            embed_model_path: PathBuf::from("/y.gguf"),
            n_gpu_layers: 0,
            ready_timeout_secs: 1,
        });
        let result = run_preflight_checks(&Commands::Run { prompt: "x".into() }, &config).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("llama-server (chat)") || err.contains("llama-server (embed)"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn preflight_ollama_skipped_for_llamacpp() {
        use crate::config::{LlamaCppConfig, LlmBackend};
        use std::path::PathBuf;

        let mut config = AppConfig::default();
        config.llm_backend = LlmBackend::LlamaCpp;
        config.ollama_host = "http://127.0.0.1:9".into();
        config.qdrant_url = "http://127.0.0.1:6334".into();
        config.llama_cpp = Some(LlamaCppConfig {
            home: PathBuf::from("/nonexistent"),
            chat_server_url: "http://127.0.0.1:9".into(),
            embed_server_url: "http://127.0.0.1:9".into(),
            chat_model_path: PathBuf::from("/x.gguf"),
            embed_model_path: PathBuf::from("/y.gguf"),
            n_gpu_layers: 0,
            ready_timeout_secs: 1,
        });
        let result = run_preflight_checks(&Commands::Run { prompt: "x".into() }, &config).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            !err.contains("Ollama daemon"),
            "Ollama must not be probed when LlamaCpp is selected: {err}"
        );
    }
}
