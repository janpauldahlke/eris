use std::io::ErrorKind;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use tokio::time::timeout;
use url::Url;

use qdrant_client::Qdrant;

use crate::config::{AppConfig, DaemonCommand, LlmBackend};

/// Ollama server default context length; must match [`AppConfig::num_ctx`] when Eris spawns `ollama serve`.
/// See <https://docs.ollama.com/context-length>.
const OLLAMA_CONTEXT_LENGTH_ENV: &str = "OLLAMA_CONTEXT_LENGTH";
use crate::executive::error::{FcpError, Result};

/// After spawning Qdrant (or Docker sidecar), poll until gRPC answers, not just TCP accept.
const READY_TIMEOUT_SECS: u64 = 45;
const READY_POLL_MS: u64 = 300;
/// Single-probe limits for [`qdrant_grpc_ready`] (tonic connect + one `health_check` RPC).
const QDRANT_PROBE_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const QDRANT_PROBE_RPC_TIMEOUT: Duration = Duration::from_secs(5);
/// If the first probe misses a slow-starting system Ollama (e.g. login item), wait this long
/// before spawning a second `ollama serve`, which duplicates RAM use.
const PRE_SPAWN_OLLAMA_WAIT_SECS: u64 = 30;
/// After SIGTERM on the managed process group, wait this long before SIGKILL.
const DAEMON_SIGTERM_GRACE_SECS: u64 = 10;
const DAEMON_TRY_WAIT_POLL_MS: u64 = 150;

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
                sync_reap_managed_child(child, self.name);
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
    llama_chat: Option<ManagedProcess>,
    llama_embed: Option<ManagedProcess>,
}

impl PeripheralLifecycle {
    pub fn started_ollama(&self) -> bool {
        self.ollama.is_some()
    }

    pub fn started_qdrant(&self) -> bool {
        self.qdrant.is_some()
    }

    pub fn started_llama_chat(&self) -> bool {
        self.llama_chat.is_some()
    }

    pub fn started_llama_embed(&self) -> bool {
        self.llama_embed.is_some()
    }

    /// Best-effort teardown without blocking the async runtime (used from chat shutdown).
    pub async fn shutdown_async(&mut self) -> Vec<&'static str> {
        let ollama = self.ollama.take();
        let qdrant = self.qdrant.take();
        let llama_embed = self.llama_embed.take();
        let llama_chat = self.llama_chat.take();
        let mut stopped = Vec::new();
        if ollama.is_some() {
            stopped.push("ollama");
        }
        if qdrant.is_some() {
            stopped.push("qdrant");
        }
        if llama_embed.is_some() {
            stopped.push("llama-embed");
        }
        if llama_chat.is_some() {
            stopped.push("llama-chat");
        }
        let join_result = tokio::task::spawn_blocking(move || {
            if let Some(mut p) = ollama {
                p.shutdown();
            }
            if let Some(mut p) = qdrant {
                p.shutdown();
            }
            // Embed first (less critical), then chat.
            if let Some(mut p) = llama_embed {
                p.shutdown();
            }
            if let Some(mut p) = llama_chat {
                p.shutdown();
            }
        })
        .await;
        if let Err(e) = join_result {
            tracing::error!(
                error = %e,
                "spawn_blocking join failed while tearing down peripheral daemons"
            );
        }
        stopped
    }

    /// Synchronous teardown (used from [`Drop`]); may block briefly.
    pub fn shutdown_started_peripherals(&mut self) -> Vec<&'static str> {
        let mut stopped = Vec::new();
        if let Some(mut ollama) = self.ollama.take() {
            ollama.shutdown();
            stopped.push("ollama");
        }
        if let Some(mut qdrant) = self.qdrant.take() {
            qdrant.shutdown();
            stopped.push("qdrant");
        }
        if let Some(mut p) = self.llama_embed.take() {
            p.shutdown();
            stopped.push("llama-embed");
        }
        if let Some(mut p) = self.llama_chat.take() {
            p.shutdown();
            stopped.push("llama-chat");
        }
        stopped
    }
}

impl Drop for PeripheralLifecycle {
    fn drop(&mut self) {
        let _ = self.shutdown_started_peripherals();
    }
}

/// Stop a [`Child`] we spawned as a dedicated process group (Unix) or the direct child only.
fn sync_reap_managed_child(child: &mut Child, name: &'static str) {
    #[cfg(unix)]
    {
        let pid = child.id();
        if pid == 0 {
            tracing::warn!(
                daemon = name,
                "child pid is 0; falling back to Child::kill (process group signal unavailable)"
            );
            if let Err(e) = child.kill() {
                tracing::warn!(daemon = name, error = %e, "Failed to stop managed daemon");
            }
            if let Err(e) = child.wait() {
                tracing::warn!(daemon = name, error = %e, "Failed to reap managed daemon");
            }
            return;
        }
        let pid_i32 = match i32::try_from(pid) {
            Ok(v) => v,
            Err(_) => {
                tracing::warn!(
                    daemon = name,
                    pid,
                    "pid does not fit i32; falling back to Child::kill (no process-group signal)"
                );
                if let Err(e) = child.kill() {
                    tracing::warn!(daemon = name, error = %e, "Failed to stop managed daemon");
                }
                if let Err(e) = child.wait() {
                    tracing::warn!(daemon = name, error = %e, "Failed to reap managed daemon");
                }
                return;
            }
        };
        let group = format!("-{pid_i32}");
        match Command::new("kill")
            .args(["-TERM", &group])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Err(e) => {
                tracing::warn!(
                    daemon = name,
                    error = %e,
                    "Could not send SIGTERM to process group; will try direct Child::kill"
                );
            }
            Ok(st) if !st.success() => {
                tracing::debug!(
                    daemon = name,
                    code = ?st.code(),
                    "SIGTERM process group returned non-success (daemon may already be exiting)"
                );
            }
            Ok(_) => {}
        }

        let deadline = std::time::Instant::now() + Duration::from_secs(DAEMON_SIGTERM_GRACE_SECS);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    tracing::info!(
                        daemon = name,
                        code = ?status.code(),
                        "Managed daemon exited after SIGTERM"
                    );
                    return;
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(
                        daemon = name,
                        error = %e,
                        "try_wait failed while waiting for managed daemon to exit"
                    );
                    break;
                }
            }
            if std::time::Instant::now() >= deadline {
                tracing::warn!(
                    daemon = name,
                    grace_secs = DAEMON_SIGTERM_GRACE_SECS,
                    "Managed daemon still running after SIGTERM grace window; sending SIGKILL to process group"
                );
                break;
            }
            std::thread::sleep(Duration::from_millis(DAEMON_TRY_WAIT_POLL_MS));
        }

        let kill_group = Command::new("kill")
            .args(["-KILL", &group])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if let Err(e) = kill_group {
            tracing::warn!(
                daemon = name,
                error = %e,
                "SIGKILL process group failed; attempting Child::kill on direct child"
            );
        }

        if let Err(e) = child.kill() {
            tracing::debug!(
                daemon = name,
                error = %e,
                "Child::kill after group SIGKILL (process may already be reaped)"
            );
        }
        match child.wait() {
            Ok(status) => {
                tracing::info!(
                    daemon = name,
                    code = ?status.code(),
                    "Managed daemon reaped"
                );
            }
            Err(e) => {
                tracing::warn!(daemon = name, error = %e, "Failed to reap managed daemon");
            }
        }
    }
    #[cfg(not(unix))]
    {
        if let Err(e) = child.kill() {
            tracing::warn!(daemon = name, error = %e, "Failed to stop managed daemon");
        }
        if let Err(e) = child.wait() {
            tracing::warn!(daemon = name, error = %e, "Failed to reap managed daemon");
        }
    }
}

/// Extract the port from a URL, returning an error if no port is present.
fn port_from_url(url: &str) -> Result<u16> {
    let parsed = Url::parse(url)
        .map_err(|e| FcpError::Config(format!("Invalid server URL '{url}': {e}")))?;
    parsed
        .port()
        .ok_or_else(|| FcpError::Config(format!("No port in server URL '{url}'")))
}

/// Poll `{url}/health` until llama-server reports `{"status":"ok"}`.
///
/// llama-server returns `{"status":"loading model"}` (HTTP 200) while loading weights —
/// we must keep polling until the status flips to `"ok"`.
async fn wait_for_llama_server(url: &str, name: &str, timeout_secs: u64) -> Result<()> {
    let health_url = format!("{}/health", url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|e| FcpError::NetworkFault(format!("HTTP client build for {name} probe: {e}")))?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);

    while tokio::time::Instant::now() < deadline {
        match client.get(&health_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                #[derive(serde::Deserialize)]
                struct HealthBody {
                    status: Option<String>,
                }
                if let Ok(body) = resp.json::<HealthBody>().await {
                    if body.status.as_deref() == Some("ok") {
                        return Ok(());
                    }
                    tracing::debug!(
                        server = name,
                        status = ?body.status,
                        "llama-server health check not ready yet"
                    );
                }
            }
            Ok(resp) => {
                tracing::debug!(
                    server = name,
                    http_status = %resp.status(),
                    "llama-server health probe got non-success HTTP status"
                );
            }
            Err(e) => {
                tracing::debug!(
                    server = name,
                    error = %e,
                    "llama-server health probe connection failed, retrying"
                );
            }
        }
        tokio::time::sleep(Duration::from_millis(READY_POLL_MS)).await;
    }

    Err(FcpError::NetworkFault(format!(
        "{name} failed to become ready within {timeout_secs}s at {url}"
    )))
}

/// Check if a llama-server instance is already responding at `url`.
async fn llama_server_already_ready(url: &str) -> bool {
    let health_url = format!("{}/health", url.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    #[derive(serde::Deserialize)]
    struct HealthBody {
        status: Option<String>,
    }

    let resp = match timeout(Duration::from_secs(2), client.get(&health_url).send()).await {
        Ok(Ok(r)) if r.status().is_success() => r,
        _ => return false,
    };
    match resp.json::<HealthBody>().await {
        Ok(body) => body.status.as_deref() == Some("ok"),
        Err(_) => false,
    }
}

impl PeripheralLifecycle {
    /// Spawn and probe both llama-server instances (chat + embed).
    ///
    /// If a server is already responding on its configured port, Eris skips spawning and
    /// treats it as an externally-managed instance (no reap on shutdown).
    pub async fn ensure_llama_servers(&mut self, config: &AppConfig) -> Result<()> {
        let lc = config.validate_llamacpp_config()?;
        let binary = lc.home.join("bin/llama-server");
        let timeout_secs = lc.ready_timeout_secs;
        let chat_ctx = config.num_ctx.max(1);
        // Embedding passes are short (chunked text). Using the full chat `num_ctx` for the
        // second managed `llama-server --embedding` duplicates KV allocation on the GPU vs
        // Ollama (one process, embed model loaded with a modest context). Cap embed context
        // so benchmarks and long-chat configs do not OOM or destabilize the display stack.
        const EMBED_SERVER_CTX_CAP: usize = 8192;
        let embed_ctx = chat_ctx.min(EMBED_SERVER_CTX_CAP);

        // --- Chat server ---
        let chat_port = port_from_url(&lc.chat_server_url)?;
        if llama_server_already_ready(&lc.chat_server_url).await {
            tracing::info!(
                server = "llama-chat",
                port = chat_port,
                "llama-server already running on port, using external instance"
            );
        } else {
            tracing::info!(
                server = "llama-chat",
                port = chat_port,
                model = %lc.chat_model_path.display(),
                enable_reasoning_fsm = config.enable_reasoning_fsm,
                "Spawning llama-server"
            );
            let mut cmd = Command::new(&binary);
            cmd.args([
                "--model",
                &lc.chat_model_path.to_string_lossy(),
                "--port",
                &chat_port.to_string(),
                "--ctx-size",
                &chat_ctx.to_string(),
                "--n-gpu-layers",
                &lc.n_gpu_layers.to_string(),
                "--log-disable",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
            // Qwen3+ chat templates: align with [`AppConfig::enable_reasoning_fsm`] / HTTP `chat_template_kwargs`.
            // Requires a recent `llama-server` that accepts `--reasoning` / `--reasoning-budget`.
            if !config.enable_reasoning_fsm {
                cmd.arg("--reasoning").arg("off");
                cmd.arg("--reasoning-budget").arg("0");
            }
            apply_unix_sidecar_process_group(&mut cmd);

            let chat_child = cmd.spawn().map_err(|e| {
                FcpError::NetworkFault(format!(
                    "Failed to spawn llama-server (chat) at {}: {e}",
                    binary.display()
                ))
            })?;
            self.llama_chat = Some(ManagedProcess {
                name: "llama-chat",
                kind: ManagedProcessKind::Child(chat_child),
            });

            if let Err(e) =
                wait_for_llama_server(&lc.chat_server_url, "llama-chat", timeout_secs).await
            {
                self.kill_llama_servers_best_effort();
                return Err(e);
            }
            tracing::info!(server = "llama-chat", "llama-server ready");
        }

        // --- Embed server ---
        let embed_port = port_from_url(&lc.embed_server_url)?;
        if llama_server_already_ready(&lc.embed_server_url).await {
            tracing::info!(
                server = "llama-embed",
                port = embed_port,
                "llama-server already running on port, using external instance"
            );
        } else {
            tracing::info!(
                server = "llama-embed",
                port = embed_port,
                model = %lc.embed_model_path.display(),
                ctx_size = embed_ctx,
                chat_ctx,
                "Spawning llama-server"
            );
            let mut cmd = Command::new(&binary);
            cmd.args([
                "--model",
                &lc.embed_model_path.to_string_lossy(),
                "--port",
                &embed_port.to_string(),
                "--embedding",
                "--ctx-size",
                &embed_ctx.to_string(),
                "--n-gpu-layers",
                &lc.n_gpu_layers.to_string(),
                "--log-disable",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
            apply_unix_sidecar_process_group(&mut cmd);

            let embed_child = cmd.spawn().map_err(|e| {
                FcpError::NetworkFault(format!(
                    "Failed to spawn llama-server (embed) at {}: {e}",
                    binary.display()
                ))
            })?;
            self.llama_embed = Some(ManagedProcess {
                name: "llama-embed",
                kind: ManagedProcessKind::Child(embed_child),
            });

            if let Err(e) =
                wait_for_llama_server(&lc.embed_server_url, "llama-embed", timeout_secs).await
            {
                self.kill_llama_servers_best_effort();
                return Err(e);
            }
            tracing::info!(server = "llama-embed", "llama-server ready");
        }

        Ok(())
    }

    /// Best-effort synchronous kill of any managed llama-server processes (used on startup failure).
    fn kill_llama_servers_best_effort(&mut self) {
        if let Some(mut p) = self.llama_chat.take() {
            p.shutdown();
        }
        if let Some(mut p) = self.llama_embed.take() {
            p.shutdown();
        }
    }
}

pub async fn ensure_peripherals_for_chat(config: &AppConfig) -> Result<PeripheralLifecycle> {
    let mut lifecycle = PeripheralLifecycle::default();

    match config.llm_backend {
        LlmBackend::Ollama => {
            if !ollama_reachable(&config.ollama_host).await {
                tracing::info!(
                    wait_secs = PRE_SPAWN_OLLAMA_WAIT_SECS,
                    "Ollama not reachable yet; waiting before attempting a managed launch"
                );
                if wait_for_ollama(&config.ollama_host, PRE_SPAWN_OLLAMA_WAIT_SECS).await {
                    tracing::info!(
                        "Ollama became reachable during pre-spawn wait; not launching a managed instance"
                    );
                } else {
                    tracing::warn!(
                        "Ollama still not reachable after extended wait; attempting Rust-managed launch"
                    );
                    let mut child = spawn_ollama_daemon(config)?;
                    if !wait_for_ollama(&config.ollama_host, READY_TIMEOUT_SECS).await {
                        sync_reap_managed_child(&mut child, "ollama-bootstrap");
                        return Err(FcpError::NetworkFault(
                            "FATAL: Ollama daemon failed to become ready after launch attempt."
                                .into(),
                        ));
                    }
                    lifecycle.ollama = Some(ManagedProcess {
                        name: "ollama",
                        kind: ManagedProcessKind::Child(child),
                    });
                    tracing::info!("Ollama launched and reachable");
                }
            }
        }
        LlmBackend::LlamaCpp => {
            lifecycle.ensure_llama_servers(config).await?;
        }
    }

    if !qdrant_grpc_ready(&config.qdrant_url).await {
        tracing::warn!("Qdrant gRPC not ready at startup, attempting Rust-managed launch");
        match spawn_daemon("qdrant", &config.qdrant_daemon) {
            Ok(mut child) => {
                if !wait_for_qdrant(&config.qdrant_url, READY_TIMEOUT_SECS).await {
                    sync_reap_managed_child(&mut child, "qdrant-bootstrap");
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
                        "FATAL: Qdrant docker sidecar failed to become ready after launch attempt."
                            .into(),
                    ));
                }
                lifecycle.qdrant = Some(ManagedProcess {
                    name: "qdrant",
                    kind: ManagedProcessKind::DockerContainer {
                        name: container_name,
                    },
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

fn apply_unix_sidecar_process_group(cmd: &mut Command) {
    #[cfg(unix)]
    {
        // New session leader: SIGTERM/SIGKILL on `-pid` reaches `ollama serve` and its runners.
        cmd.process_group(0);
    }
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
    let mut cmd = Command::new(&daemon.command);
    cmd.args(&daemon.args)
        .env(OLLAMA_CONTEXT_LENGTH_ENV, ctx.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    apply_unix_sidecar_process_group(&mut cmd);
    cmd.spawn().map_err(|e| {
        FcpError::NetworkFault(format!(
            "FATAL: failed to launch ollama daemon with `{}` ({}={}): {e}",
            render_daemon_command(daemon),
            OLLAMA_CONTEXT_LENGTH_ENV,
            ctx
        ))
    })
}

fn spawn_daemon(name: &'static str, daemon: &DaemonCommand) -> Result<Child> {
    let mut cmd = Command::new(&daemon.command);
    cmd.args(&daemon.args)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    apply_unix_sidecar_process_group(&mut cmd);
    cmd.spawn().map_err(|e| {
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

/// Best-effort `ollama stop` for the configured chat and embedding models. Frees most model RAM
/// while leaving the Ollama server (e.g. Ollama.app) running. Safe to call when no model is loaded.
pub async fn unload_ollama_models_cli_best_effort(config: &AppConfig) {
    if !crate::util::ollama_host_cli::host_ollama_cli_subprocess_allowed() {
        tracing::debug!(
            event = "fcp.ollama.unload_skipped",
            "Skipping `ollama stop` (host CLI disabled under test/CI or FCP_SKIP_HOST_OLLAMA_CLI)"
        );
        return;
    }
    let mut models = vec![config.model_name.clone(), config.embed_model_name.clone()];
    models.retain(|m| !m.trim().is_empty());
    models.sort();
    models.dedup();
    if models.is_empty() {
        return;
    }
    let join_result = tokio::task::spawn_blocking(move || {
        for model in models {
            let status = Command::new("ollama")
                .args(["stop", &model])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            match status {
                Ok(s) if s.success() => {
                    tracing::info!(%model, "ollama stop succeeded after chat exit");
                }
                Ok(s) => {
                    tracing::debug!(
                        %model,
                        code = ?s.code(),
                        "ollama stop exited with non-success (model may not have been loaded)"
                    );
                }
                Err(e) => {
                    tracing::debug!(
                        %model,
                        error = %e,
                        "ollama CLI not available or stop invocation failed"
                    );
                }
            }
        }
    })
    .await;
    if let Err(e) = join_result {
        tracing::warn!(
            error = %e,
            "spawn_blocking join failed while unloading Ollama models"
        );
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

/// `GET {base_url}/health` with a short timeout (llama-server readiness).
pub async fn llama_server_reachable(base_url: &str) -> bool {
    let client = reqwest::Client::new();
    let health_url = format!("{}/health", base_url.trim_end_matches('/'));
    matches!(
        timeout(Duration::from_secs(2), client.get(health_url).send()).await,
        Ok(Ok(res)) if res.status().is_success()
    )
}

/// True when Qdrant serves gRPC on `qdrant_url` (same readiness signal as the semantic brain).
///
/// Uses `skip_compatibility_check` so repeated polls do not spam stdout from `qdrant-client`.
pub async fn qdrant_grpc_ready(qdrant_url: &str) -> bool {
    if parse_socket_addr(qdrant_url, 6334).is_err() {
        return false;
    }
    let client = match Qdrant::from_url(qdrant_url)
        .skip_compatibility_check()
        .connect_timeout(QDRANT_PROBE_CONNECT_TIMEOUT)
        .timeout(QDRANT_PROBE_RPC_TIMEOUT)
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    matches!(
        timeout(QDRANT_PROBE_RPC_TIMEOUT, client.health_check()).await,
        Ok(Ok(_))
    )
}

async fn wait_for_ollama(host: &str, timeout_secs: u64) -> bool {
    wait_until(timeout_secs, || ollama_reachable(host)).await
}

async fn wait_for_qdrant(url: &str, timeout_secs: u64) -> bool {
    wait_until(timeout_secs, || qdrant_grpc_ready(url)).await
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

    #[tokio::test]
    async fn qdrant_grpc_ready_false_on_dead_port() {
        assert!(
            !qdrant_grpc_ready("http://127.0.0.1:65535").await,
            "no listener on ephemeral port"
        );
    }

    #[tokio::test]
    async fn qdrant_grpc_ready_false_on_invalid_url() {
        assert!(!qdrant_grpc_ready("not-a-url").await);
    }

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

    // ── Phase 2: llama-server process management tests ──

    #[test]
    fn port_from_url_parses_correctly() {
        assert_eq!(port_from_url("http://127.0.0.1:8090").unwrap(), 8090);
        assert_eq!(port_from_url("http://localhost:8091").unwrap(), 8091);
        assert_eq!(port_from_url("http://0.0.0.0:9999").unwrap(), 9999);
    }

    #[test]
    fn port_from_url_missing_port_errors() {
        let err = port_from_url("http://localhost").unwrap_err();
        assert!(matches!(err, FcpError::Config(_)));
        assert!(err.to_string().contains("No port"));
    }

    #[test]
    fn port_from_url_invalid_url_errors() {
        let err = port_from_url("not-a-url").unwrap_err();
        assert!(matches!(err, FcpError::Config(_)));
        assert!(err.to_string().contains("Invalid server URL"));
    }

    #[tokio::test]
    async fn ready_probe_succeeds_on_healthy_server() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = format!("http://127.0.0.1:{port}");

        tokio::spawn(async move {
            loop {
                let (mut stream, _) = match listener.accept().await {
                    Ok(v) => v,
                    Err(_) => break,
                };
                use tokio::io::AsyncWriteExt;
                let body = r#"{"status":"ok"}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });

        let result = wait_for_llama_server(&url, "test-chat", 5).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn ready_probe_waits_for_loading() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = format!("http://127.0.0.1:{port}");
        let request_count = std::sync::Arc::new(AtomicU32::new(0));
        let count = request_count.clone();

        tokio::spawn(async move {
            loop {
                let (mut stream, _) = match listener.accept().await {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let n = count.fetch_add(1, Ordering::SeqCst);
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf).await;

                let body = if n < 2 {
                    r#"{"status":"loading model"}"#
                } else {
                    r#"{"status":"ok"}"#
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });

        let result = wait_for_llama_server(&url, "test-loading", 10).await;
        assert!(result.is_ok());
        assert!(request_count.load(Ordering::SeqCst) >= 3);
    }

    #[tokio::test]
    async fn ready_probe_timeout_returns_error() {
        // Use a port that nothing is listening on.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let url = format!("http://127.0.0.1:{port}");
        let result = wait_for_llama_server(&url, "test-timeout", 1).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("failed to become ready"));
    }

    #[tokio::test]
    async fn shutdown_reaps_both_processes() {
        let child1 = Command::new("sleep")
            .arg("300")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid1 = child1.id();

        let child2 = Command::new("sleep")
            .arg("300")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid2 = child2.id();

        let mut lifecycle = PeripheralLifecycle {
            ollama: None,
            qdrant: None,
            llama_chat: Some(ManagedProcess {
                name: "llama-chat",
                kind: ManagedProcessKind::Child(child1),
            }),
            llama_embed: Some(ManagedProcess {
                name: "llama-embed",
                kind: ManagedProcessKind::Child(child2),
            }),
        };

        let stopped = lifecycle.shutdown_async().await;
        assert!(stopped.contains(&"llama-chat"));
        assert!(stopped.contains(&"llama-embed"));

        // Verify processes are gone (kill(pid, 0) should fail).
        #[cfg(unix)]
        {
            use std::process::Command as StdCmd;
            for pid in [pid1, pid2] {
                let status = StdCmd::new("kill")
                    .args(["-0", &pid.to_string()])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                assert!(
                    status.map(|s| !s.success()).unwrap_or(true),
                    "Process {pid} should have been reaped"
                );
            }
        }
    }

    #[test]
    fn llama_accessors_reflect_state() {
        let lifecycle = PeripheralLifecycle::default();
        assert!(!lifecycle.started_llama_chat());
        assert!(!lifecycle.started_llama_embed());

        let child = Command::new("true")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut lifecycle = PeripheralLifecycle {
            ollama: None,
            qdrant: None,
            llama_chat: Some(ManagedProcess {
                name: "llama-chat",
                kind: ManagedProcessKind::Child(child),
            }),
            llama_embed: None,
        };
        assert!(lifecycle.started_llama_chat());
        assert!(!lifecycle.started_llama_embed());

        // Clean up so we don't leak the process.
        lifecycle.shutdown_started_peripherals();
    }
}
