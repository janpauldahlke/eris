use std::process::{Child, Command, Stdio};
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::time::timeout;
use url::Url;

use crate::config::{AppConfig, DaemonCommand};
use crate::executive::error::{FcpError, Result};

const READY_TIMEOUT_SECS: u64 = 20;
const READY_POLL_MS: u64 = 250;

struct ManagedProcess {
    name: &'static str,
    child: Child,
}

impl ManagedProcess {
    fn shutdown(&mut self) {
        if let Err(e) = self.child.kill() {
            tracing::warn!(daemon = self.name, error = %e, "Failed to stop managed daemon");
        }
        if let Err(e) = self.child.wait() {
            tracing::warn!(daemon = self.name, error = %e, "Failed to reap managed daemon");
        }
    }
}

#[derive(Default)]
pub struct PeripheralLifecycle {
    ollama: Option<ManagedProcess>,
    qdrant: Option<ManagedProcess>,
}

impl PeripheralLifecycle {
    pub fn shutdown_started_peripherals(&mut self) {
        if let Some(ollama) = self.ollama.as_mut() {
            ollama.shutdown();
        }
        if let Some(qdrant) = self.qdrant.as_mut() {
            qdrant.shutdown();
        }
    }
}

impl Drop for PeripheralLifecycle {
    fn drop(&mut self) {
        self.shutdown_started_peripherals();
    }
}

pub async fn ensure_peripherals_for_chat(config: &AppConfig) -> Result<PeripheralLifecycle> {
    let mut lifecycle = PeripheralLifecycle::default();

    if !ollama_reachable(&config.ollama_host).await {
        tracing::warn!("Ollama not reachable at startup, attempting Rust-managed launch");
        let mut child = spawn_daemon("ollama", &config.ollama_daemon)?;
        if !wait_for_ollama(&config.ollama_host, READY_TIMEOUT_SECS).await {
            let _ = child.kill();
            let _ = child.wait();
            return Err(FcpError::NetworkFault(
                "FATAL: Ollama daemon failed to become ready after launch attempt.".into(),
            ));
        }
        lifecycle.ollama = Some(ManagedProcess {
            name: "ollama",
            child,
        });
        tracing::info!("Ollama launched and reachable");
    }

    if !qdrant_reachable(&config.qdrant_url).await {
        tracing::warn!("Qdrant not reachable at startup, attempting Rust-managed launch");
        let mut child = spawn_daemon("qdrant", &config.qdrant_daemon)?;
        if !wait_for_qdrant(&config.qdrant_url, READY_TIMEOUT_SECS).await {
            let _ = child.kill();
            let _ = child.wait();
            return Err(FcpError::NetworkFault(
                "FATAL: Qdrant sidecar failed to become ready after launch attempt.".into(),
            ));
        }
        lifecycle.qdrant = Some(ManagedProcess {
            name: "qdrant",
            child,
        });
        tracing::info!("Qdrant launched and reachable");
    }

    Ok(lifecycle)
}

fn spawn_daemon(name: &'static str, daemon: &DaemonCommand) -> Result<Child> {
    Command::new(&daemon.command)
        .args(&daemon.args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            FcpError::NetworkFault(format!(
                "FATAL: failed to launch {name} daemon with `{}`: {e}",
                render_daemon_command(daemon)
            ))
        })
}

fn render_daemon_command(daemon: &DaemonCommand) -> String {
    if daemon.args.is_empty() {
        daemon.command.clone()
    } else {
        format!("{} {}", daemon.command, daemon.args.join(" "))
    }
}

pub async fn ollama_reachable(ollama_host: &str) -> bool {
    let client = reqwest::Client::new();
    let tags_url = format!("{}/api/tags", ollama_host.trim_end_matches('/'));
    matches!(
        timeout(Duration::from_secs(2), client.get(tags_url).send()).await,
        Ok(Ok(res)) if res.status().is_success()
    )
}

pub async fn qdrant_reachable(qdrant_url: &str) -> bool {
    let addr = match parse_socket_addr(qdrant_url, 6334) {
        Ok(v) => v,
        Err(_) => return false,
    };
    matches!(
        timeout(Duration::from_secs(2), TcpStream::connect(addr)).await,
        Ok(Ok(_))
    )
}

async fn wait_for_ollama(host: &str, timeout_secs: u64) -> bool {
    wait_until(timeout_secs, || ollama_reachable(host)).await
}

async fn wait_for_qdrant(url: &str, timeout_secs: u64) -> bool {
    wait_until(timeout_secs, || qdrant_reachable(url)).await
}

async fn wait_until<F, Fut>(timeout_secs: u64, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
    while tokio::time::Instant::now() < deadline {
        if check().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(READY_POLL_MS)).await;
    }
    false
}

pub fn parse_socket_addr(service_url: &str, default_port: u16) -> Result<String> {
    let parsed = Url::parse(service_url)
        .map_err(|e| FcpError::Config(format!("Invalid service URL `{service_url}`: {e}")))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| FcpError::Config(format!("Service URL missing host: {service_url}")))?;
    let port = parsed.port().unwrap_or(default_port);
    Ok(format!("{host}:{port}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_socket_addr_works() {
        let addr = parse_socket_addr("http://localhost:6334", 6334).unwrap();
        assert_eq!(addr, "localhost:6334");
        let addr2 = parse_socket_addr("https://example.com", 9999).unwrap();
        assert_eq!(addr2, "example.com:9999");
    }

    #[test]
    fn parse_socket_addr_rejects_invalid_input() {
        let result = parse_socket_addr("not-a-url", 6334);
        assert!(result.is_err());
    }

    #[test]
    fn render_daemon_command_works() {
        let command = DaemonCommand {
            command: "ollama".into(),
            args: vec!["serve".into()],
        };
        assert_eq!(render_daemon_command(&command), "ollama serve");
    }
}
