//! Async snapshot of third-party tools for the first-run welder.

use crate::config::AppConfig;
use crate::executive::peripherals;

/// Outcome of probing the local environment (no installs).
#[derive(Debug, Clone)]
pub struct WelderReport {
    pub ollama_api_ok: bool,
    pub qdrant_tcp_ok: bool,
    pub ollama_cli: bool,
    pub qdrant_cli: bool,
    pub docker_cli: bool,
    pub require_semantic_brain: bool,
}

async fn path_command_ok(program: &'static str, args: &[&'static str]) -> bool {
    let program_owned = program.to_string();
    let args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    match tokio::task::spawn_blocking(move || {
        std::process::Command::new(&program_owned)
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_or(false, |s| s.success())
    })
    .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, program, "path probe task join failed");
            false
        }
    }
}

/// Collect reachability and PATH probes (Ollama/Qdrant are cheap HTTP/TCP).
pub async fn gather(config: &AppConfig) -> WelderReport {
    let ollama_api_ok = peripherals::ollama_reachable(&config.ollama_host).await;
    let qdrant_tcp_ok = peripherals::qdrant_reachable(&config.qdrant_url).await;
    let require_semantic_brain = config.require_semantic_brain;

    let (ollama_cli, qdrant_cli, docker_cli) = tokio::join!(
        path_command_ok("ollama", &["--version"]),
        path_command_ok("qdrant", &["--version"]),
        path_command_ok("docker", &["--version"]),
    );

    WelderReport {
        ollama_api_ok,
        qdrant_tcp_ok,
        ollama_cli,
        qdrant_cli,
        docker_cli,
        require_semantic_brain,
    }
}

impl WelderReport {
    /// Qdrant can still be auto-started if native binary or Docker exists.
    pub fn qdrant_can_be_auto_started(&self) -> bool {
        self.qdrant_tcp_ok || self.qdrant_cli || self.docker_cli
    }

    /// Hard preflight: semantic brain required but no path to start Qdrant.
    pub fn qdrant_blocked(&self) -> bool {
        self.require_semantic_brain && !self.qdrant_can_be_auto_started()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qdrant_blocked_semantics() {
        let r = WelderReport {
            ollama_api_ok: false,
            qdrant_tcp_ok: false,
            ollama_cli: false,
            qdrant_cli: false,
            docker_cli: false,
            require_semantic_brain: true,
        };
        assert!(r.qdrant_blocked());
        let r2 = WelderReport {
            require_semantic_brain: true,
            docker_cli: true,
            ..r
        };
        assert!(!r2.qdrant_blocked());
    }
}
