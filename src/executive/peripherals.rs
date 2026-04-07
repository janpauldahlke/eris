use std::io::ErrorKind;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::time::timeout;
use url::Url;

use crate::config::{AppConfig, DaemonCommand};

/// Ollama server default context length; must match [`AppConfig::num_ctx`] when Eris spawns `ollama serve`.
/// See <https://docs.ollama.com/context-length>.
const OLLAMA_CONTEXT_LENGTH_ENV: &str = "OLLAMA_CONTEXT_LENGTH";
use crate::executive::error::{FcpError, Result};

const READY_TIMEOUT_SECS: u64 = 20;
const READY_POLL_MS: u64 = 250;
/// If the first probe misses a slow-starting system Ollama (e.g. login item), wait this long
/// before spawning a second `ollama serve`, which duplicates RAM use.
const PRE_SPAWN_OLLAMA_WAIT_SECS: u64 = 30;

enum ManagedProcessKind {
    Child(Child),
    DockerContainer { name: String },
}

struct ManagedProcess {
    name: &'static str,
    kind: ManagedProcessKind,
}

impl ManagedProcess {
    fn shutdown(&mut self) {
        match &mut self.kind {
            ManagedProcessKind::Child(child) => {
                if let Err(e) = child.kill() {
                    tracing::warn!(daemon = self.name, error = %e, "Failed to stop managed daemon");
                }
                if let Err(e) = child.wait() {
                    tracing::warn!(daemon = self.name, error = %e, "Failed to reap managed daemon");
                }
            }
            ManagedProcessKind::DockerContainer { name } => {
                let status = Command::new("docker")
                    .args(["stop", name])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                if let Err(e) = status {
                    tracing::warn!(daemon = self.name, container = %name, error = %e, "Failed to stop managed docker daemon");
                }
            }
        }
    }
}

#[derive(Default)]
pub struct PeripheralLifecycle {
    ollama: Option<ManagedProcess>,
    qdrant: Option<ManagedProcess>,
}

impl PeripheralLifecycle {
    pub fn started_ollama(&self) -> bool {
        self.ollama.is_some()
    }

    pub fn started_qdrant(&self) -> bool {
        self.qdrant.is_some()
    }

    pub fn shutdown_started_peripherals(&mut self) -> Vec<&'static str> {
        let mut stopped = Vec::new();
        if let Some(ollama) = self.ollama.as_mut() {
            ollama.shutdown();
            stopped.push("ollama");
        }
        if let Some(qdrant) = self.qdrant.as_mut() {
            qdrant.shutdown();
            stopped.push("qdrant");
        }
        stopped
    }
}

impl Drop for PeripheralLifecycle {
    fn drop(&mut self) {
        let _ = self.shutdown_started_peripherals();
    }
}

pub async fn ensure_peripherals_for_chat(config: &AppConfig) -> Result<PeripheralLifecycle> {
    let mut lifecycle = PeripheralLifecycle::default();

    if !ollama_reachable(&config.ollama_host).await {
        tracing::info!(
            wait_secs = PRE_SPAWN_OLLAMA_WAIT_SECS,
            "Ollama not reachable yet; waiting before attempting a managed launch"
        );
        if wait_for_ollama(&config.ollama_host, PRE_SPAWN_OLLAMA_WAIT_SECS).await {
            tracing::info!("Ollama became reachable during pre-spawn wait; not launching a managed instance");
        } else {
            tracing::warn!("Ollama still not reachable after extended wait; attempting Rust-managed launch");
            let mut child = spawn_ollama_daemon(config)?;
            if !wait_for_ollama(&config.ollama_host, READY_TIMEOUT_SECS).await {
                let _ = child.kill();
                let _ = child.wait();
                return Err(FcpError::NetworkFault(
                    "FATAL: Ollama daemon failed to become ready after launch attempt.".into(),
                ));
            }
            lifecycle.ollama = Some(ManagedProcess {
                name: "ollama",
                kind: ManagedProcessKind::Child(child),
            });
            tracing::info!("Ollama launched and reachable");
        }
    }

    if !qdrant_reachable(&config.qdrant_url).await {
        tracing::warn!("Qdrant not reachable at startup, attempting Rust-managed launch");
        match spawn_daemon("qdrant", &config.qdrant_daemon) {
            Ok(mut child) => {
                if !wait_for_qdrant(&config.qdrant_url, READY_TIMEOUT_SECS).await {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(FcpError::NetworkFault(
                        "FATAL: Qdrant sidecar failed to become ready after launch attempt.".into(),
                    ));
                }
                lifecycle.qdrant = Some(ManagedProcess {
                    name: "qdrant",
                    kind: ManagedProcessKind::Child(child),
                });
                tracing::info!("Qdrant launched and reachable");
            }
            Err(FcpError::NetworkFault(msg))
                if is_default_qdrant_command(&config.qdrant_daemon)
                    && msg.contains("No such file or directory") =>
            {
                let container_name = start_qdrant_via_docker(&config.qdrant_url)?;
                if !wait_for_qdrant(&config.qdrant_url, READY_TIMEOUT_SECS).await {
                    let _ = Command::new("docker")
                        .args(["stop", &container_name])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                    return Err(FcpError::NetworkFault(
                        "FATAL: Qdrant docker sidecar failed to become ready after launch attempt.".into(),
                    ));
                }
                lifecycle.qdrant = Some(ManagedProcess {
                    name: "qdrant",
                    kind: ManagedProcessKind::DockerContainer { name: container_name },
                });
                tracing::info!("Qdrant docker sidecar launched and reachable");
            }
            Err(e) => return Err(e),
        }
    }

    Ok(lifecycle)
}

fn is_default_qdrant_command(daemon: &DaemonCommand) -> bool {
    daemon.command == "qdrant" && daemon.args.is_empty()
}

fn start_qdrant_via_docker(qdrant_url: &str) -> Result<String> {
    let container_name = "eris-qdrant-sidecar".to_string();
    let host_port = qdrant_host_port(qdrant_url)?;

    let docker_exists = Command::new("docker")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !docker_exists {
        return Err(FcpError::NetworkFault(
            "FATAL: qdrant binary not found and docker is unavailable; cannot auto-start qdrant sidecar.".into(),
        ));
    }

    let existing = Command::new("docker")
        .args([
            "ps",
            "-a",
            "--filter",
            "name=^/eris-qdrant-sidecar$",
            "--format",
            "{{.Names}}",
        ])
        .output()?;
    let has_existing = String::from_utf8_lossy(&existing.stdout).trim() == container_name;

    let status = if has_existing {
        Command::new("docker")
            .args(["start", &container_name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    } else {
        Command::new("docker")
            .args([
                "run",
                "-d",
                "--rm",
                "--name",
                &container_name,
                "-p",
                &format!("{host_port}:6334"),
                "qdrant/qdrant:latest",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    };

    match status {
        Ok(s) if s.success() => Ok(container_name),
        Ok(s) => Err(FcpError::NetworkFault(format!(
            "FATAL: failed to launch qdrant docker sidecar (exit status: {s})"
        ))),
        Err(e) if e.kind() == ErrorKind::NotFound => Err(FcpError::NetworkFault(
            "FATAL: docker executable not found; cannot auto-start qdrant sidecar.".into(),
        )),
        Err(e) => Err(FcpError::NetworkFault(format!(
            "FATAL: failed to launch qdrant docker sidecar: {e}"
        ))),
    }
}

fn qdrant_host_port(qdrant_url: &str) -> Result<u16> {
    let parsed = Url::parse(qdrant_url)
        .map_err(|e| FcpError::Config(format!("Invalid qdrant_url `{qdrant_url}`: {e}")))?;
    Ok(parsed.port().unwrap_or(6334))
}

fn spawn_ollama_daemon(config: &AppConfig) -> Result<Child> {
    let daemon = &config.ollama_daemon;
    let ctx = config.num_ctx.max(1);
    tracing::info!(
        num_ctx = ctx,
        env = OLLAMA_CONTEXT_LENGTH_ENV,
        command = %render_daemon_command(daemon),
        "Spawning Ollama with server default context length from config"
    );
    Command::new(&daemon.command)
        .args(&daemon.args)
        .env(OLLAMA_CONTEXT_LENGTH_ENV, ctx.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            FcpError::NetworkFault(format!(
                "FATAL: failed to launch ollama daemon with `{}` ({}={}): {e}",
                render_daemon_command(daemon),
                OLLAMA_CONTEXT_LENGTH_ENV,
                ctx
            ))
        })
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

    #[test]
    fn default_qdrant_command_detection() {
        let default = DaemonCommand {
            command: "qdrant".into(),
            args: Vec::new(),
        };
        assert!(is_default_qdrant_command(&default));

        let custom = DaemonCommand {
            command: "docker".into(),
            args: vec!["run".into()],
        };
        assert!(!is_default_qdrant_command(&custom));
    }

    #[test]
    fn qdrant_host_port_uses_configured_port() {
        assert_eq!(qdrant_host_port("http://localhost:7123").unwrap(), 7123);
        assert_eq!(qdrant_host_port("http://localhost").unwrap(), 6334);
    }
}
