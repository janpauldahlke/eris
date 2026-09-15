use crate::config::{AppConfig, EmbedBackend, LlmBackend};
use crate::executive::cli::Commands;
use crate::executive::error::{FcpError, Result};
use crate::executive::peripherals::{llama_server_reachable, ollama_reachable, qdrant_grpc_ready};

/// Free OpenRouter reachability/auth probe: `GET {base_url}/key` validates the key without a
/// paid generation. Never logs the key; only the HTTP status is surfaced.
async fn openrouter_key_probe(base_url: &str, api_key: &str) -> Result<()> {
    let url = format!("{}/key", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| FcpError::NetworkFault(format!("HTTP client build: {e}")))?;
    let resp = client
        .get(&url)
        .bearer_auth(api_key.trim())
        .send()
        .await
        .map_err(|e| FcpError::NetworkFault(format!("OpenRouter unreachable at {url}: {e}")))?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(FcpError::Config(
            "OpenRouter rejected the API key (HTTP 401). Check the key env var.".into(),
        ));
    }
    if !status.is_success() {
        return Err(FcpError::NetworkFault(format!(
            "OpenRouter key probe at {url} returned HTTP {status}"
        )));
    }
    Ok(())
}

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
        LlmBackend::OpenRouter => {
            let or = config.validate_openrouter_config()?;
            let api_key = std::env::var(&or.api_key_env).map_err(|_| {
                FcpError::Config(format!(
                    "OpenRouter API key env var `{}` is not set",
                    or.api_key_env
                ))
            })?;
            openrouter_key_probe(&or.base_url, &api_key).await?;
            // Local embed backend must still be reachable.
            match config.resolved_embed_backend() {
                EmbedBackend::Ollama => {
                    if !ollama_reachable(&config.ollama_host).await {
                        return Err(FcpError::NetworkFault(
                            "FATAL: Ollama daemon (embeddings) not responding.".into(),
                        ));
                    }
                }
                EmbedBackend::LlamaCpp => {
                    if let Some(lc) = config.llama_cpp.as_ref() {
                        if !llama_server_reachable(&lc.embed_server_url).await {
                            return Err(FcpError::NetworkFault(format!(
                                "FATAL: llama-server (embed) not responding at {}",
                                lc.embed_server_url
                            )));
                        }
                    }
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

    #[tokio::test(flavor = "current_thread")]
    async fn preflight_skips_chat_mode() {
        let mut config = AppConfig::default();
        config.ollama_host = "not a url".into();
        config.qdrant_url = "still-not-a-url".into();
        let result = run_preflight_checks(&Commands::Chat { web: false }, &config).await;
        assert!(result.is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn preflight_checks_non_chat_mode() {
        let mut config = AppConfig::default();
        config.ollama_host = "http://127.0.0.1:9".into();
        let result = run_preflight_checks(&Commands::Run { prompt: "x".into() }, &config).await;
        assert!(result.is_err());
    }

    #[tokio::test(flavor = "current_thread")]
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
            ready_timeout_secs: 1,
            ..Default::default()
        });
        let result = run_preflight_checks(&Commands::Run { prompt: "x".into() }, &config).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("llama-server (chat)") || err.contains("llama-server (embed)"),
            "{err}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
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
            ready_timeout_secs: 1,
            ..Default::default()
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
