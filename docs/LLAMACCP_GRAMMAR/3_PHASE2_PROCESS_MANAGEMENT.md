# Phase 2 — Process Management for llama-server

**Depends on:** Phase 0 (config), Phase 1 (client exists to use the servers)
**Unlocks:** Phase 3 (embedding needs the embed server running), Phase 6 (health probes)
**Estimated scope:** ~250 LOC new/modified, ~6 tests

---

## 2.1 — Goal

Eris manages `llama-server` lifecycle the same way it manages Ollama via `PeripheralLifecycle`. Two processes: **chat server** (port 8090) and **embed server** (port 8091). On `eris chat` startup, both are spawned, probed for readiness, and torn down on exit.

---

## 2.2 — `PeripheralLifecycle` Changes (`src/executive/peripherals.rs`)

### 2.2.1 New Fields

```rust
pub struct PeripheralLifecycle {
    ollama: Option<ManagedProcess>,
    qdrant: Option<ManagedProcess>,
    llama_chat: Option<ManagedProcess>,    // NEW
    llama_embed: Option<ManagedProcess>,   // NEW
}
```

### 2.2.2 New Public Methods

```rust
impl PeripheralLifecycle {
    pub fn started_llama_chat(&self) -> bool {
        self.llama_chat.is_some()
    }
    pub fn started_llama_embed(&self) -> bool {
        self.llama_embed.is_some()
    }
}
```

### 2.2.3 Spawn Logic

Add a new public async method:

```rust
pub async fn ensure_llama_servers(&mut self, config: &AppConfig) -> Result<()>
```

This method:

1. **Validates** `config.validate_llamacpp_config()?` to get `LlamaCppConfig`.

2. **Spawns the chat server:**

   ```
   {home}/bin/llama-server \
       --model {chat_model_path} \
       --port {port_from_chat_server_url} \
       --ctx-size {num_ctx} \
       --n-gpu-layers {n_gpu_layers} \
       --log-disable
   ```

   - Extract port from `chat_server_url` via `url::Url` parsing (default 8090)
   - `--ctx-size` uses top-level `AppConfig.num_ctx` (same as Ollama `num_ctx` and orchestrator budgets)
   - `--log-disable` keeps llama-server stdout clean (logs go to Eris tracing instead)
   - Pipe `stdout` and `stderr` to `Stdio::null()` (or to a file in `.fcp/logs/` if we want llama-server logs)
   - Use `Command::new(...).process_group(0)` on Unix so SIGTERM reaps the group (same pattern as Ollama)

3. **Spawns the embed server:**

   ```
   {home}/bin/llama-server \
       --model {embed_model_path} \
       --port {port_from_embed_server_url} \
       --embedding \
       --ctx-size {num_ctx} \
       --n-gpu-layers {n_gpu_layers} \
       --log-disable
   ```

   - `--embedding` flag enables the `/v1/embeddings` endpoint
   - `--ctx-size` matches `AppConfig.num_ctx` (same knob as chat server)

4. **Readiness probe** for each server (see §2.3).

5. **On failure:** If either server fails to start or passes readiness timeout:
   - Kill the other if it started
   - Return descriptive `FcpError` with the server name, expected port, and failure reason

### 2.2.4 Port Extraction Helper

```rust
fn port_from_url(url: &str) -> Result<u16> {
    let parsed = Url::parse(url)
        .map_err(|e| FcpError::Config(format!("Invalid server URL '{url}': {e}")))?;
    parsed.port().ok_or_else(|| FcpError::Config(
        format!("No port in server URL '{url}'")
    ))
}
```

---

## 2.3 — Readiness Probing

Reuse the existing probe pattern from `wait_for_tcp_port`:

### 2.3.1 TCP Connect + HTTP Health Check

```rust
async fn wait_for_llama_server(url: &str, name: &str) -> Result<()>
```

1. Parse host and port from `url`.
2. Poll loop: every `READY_POLL_MS` (250ms), attempt `TcpStream::connect`.
3. Once TCP connects, GET `{url}/health`.
4. Expect HTTP 200 with body `{"status":"ok"}`.
5. If `READY_TIMEOUT_SECS` (20s) passes without success, return error.

**llama-server specifics:**

- llama-server reports `{"status":"loading model"}` while loading weights — this is a 200 response but NOT ready. Check that `status == "ok"`.
- During GGUF quantized model loading, the server can take 5-15 seconds. The 20s timeout should be sufficient for most models, but consider making this configurable in `LlamaCppConfig` as `ready_timeout_secs` with default 30.

### 2.3.2 Consider Adding `ready_timeout_secs` to `LlamaCppConfig`

```rust
/// Max seconds to wait for each llama-server to become ready after spawn.
#[serde(default = "default_llamacpp_ready_timeout")]
pub ready_timeout_secs: u64,
```

Default: 30 seconds (larger models on CPU need more time).

---

## 2.4 — Shutdown Logic

### 2.4.1 Integration with `shutdown_async`

Extend the existing `shutdown_async` method:

```rust
pub async fn shutdown_async(&mut self) -> Vec<&'static str> {
    let ollama = self.ollama.take();
    let qdrant = self.qdrant.take();
    let llama_chat = self.llama_chat.take();     // NEW
    let llama_embed = self.llama_embed.take();   // NEW
    let mut stopped = Vec::new();
    // ... existing ollama/qdrant shutdown ...
    if let Some(mut p) = llama_chat {
        p.shutdown();
        stopped.push("llama-chat");
    }
    if let Some(mut p) = llama_embed {
        p.shutdown();
        stopped.push("llama-embed");
    }
    stopped
}
```

### 2.4.2 Shutdown Ordering

Kill embed server first (it's less critical), then chat server. Both use the existing `sync_reap_managed_child` pattern: SIGTERM → wait `DAEMON_SIGTERM_GRACE_SECS` (10s) → SIGKILL.

---

## 2.5 — Chat Session Integration

### 2.5.1 `src/executive/chat_session.rs` Changes

Peripheral startup currently calls the free function `ensure_peripherals_for_chat(&config)` which returns a `PeripheralLifecycle`. This function internally handles both Ollama and Qdrant. It needs to be extended with a backend branch:

```rust
// Current call site:
let peripheral_lifecycle = ensure_peripherals_for_chat(&config).await?;
```

Inside `ensure_peripherals_for_chat` (in `peripherals.rs`), add the backend branch:

```rust
match config.llm_backend {
    LlmBackend::Ollama => {
        // existing Ollama spawn/probe logic (unchanged)
    }
    LlmBackend::LlamaCpp => {
        lifecycle.ensure_llama_servers(config).await?;
    }
}
// Qdrant logic stays unconditional (both backends need it)
```

### 2.5.2 Skip Ollama When llama.cpp

Important: When `LlmBackend::LlamaCpp`, do NOT run the Ollama spawn/probe logic. Ollama is not needed — the embed server handles embeddings, and the chat server handles generation.

However, **Qdrant is still needed** for semantic memory regardless of backend. The Qdrant spawn/probe must remain for both paths.

---

## 2.6 — External vs. Managed Mode

### 2.6.1 User-Managed llama-server

Some users will prefer to manage `llama-server` themselves (e.g., running it as a systemd service, or in a tmux). Support this by:

- If `llama-server` is already responding on the configured port when Eris starts, **skip spawning** and log `info!("llama-server already running on port {port}, using external instance")`
- On shutdown, do NOT kill an externally-managed server (only kill `ManagedProcess` children Eris spawned)

Detection: Before spawning, attempt the readiness probe. If it succeeds immediately, set `llama_chat = None` (no managed process to reap).

### 2.6.2 Config Option (Optional)

Consider a `managed: bool` field in `LlamaCppConfig` (default `true`). When `false`, Eris never spawns — only connects. This avoids the ambiguity of "was it already running or did we spawn it?".

---

## 2.7 — Logging and Tracing

All lifecycle events use `tracing`:

```rust
tracing::info!(server = "llama-chat", port = port, model = %model_path.display(), "Spawning llama-server");
tracing::info!(server = "llama-chat", elapsed_ms = elapsed, "llama-server ready");
tracing::warn!(server = "llama-embed", error = %e, "llama-server health check failed, retrying");
tracing::error!(server = "llama-chat", "llama-server failed to start within timeout");
```

No `println!` — it corrupts the ratatui buffer.

---

## 2.8 — Tests

| #   | Test name                                | What it validates                                                                      |
| --- | ---------------------------------------- | -------------------------------------------------------------------------------------- |
| 1   | `port_from_url_parses_correctly`         | `http://127.0.0.1:8090` → 8090                                                         |
| 2   | `port_from_url_missing_port_errors`      | `http://localhost` → `FcpError::Config`                                                |
| 3   | `ready_probe_succeeds_on_healthy_server` | Mock HTTP server returns `{"status":"ok"}` → success                                   |
| 4   | `ready_probe_waits_for_loading`          | Mock returns `{"status":"loading model"}` then `{"status":"ok"}` → success after retry |
| 5   | `ready_probe_timeout_returns_error`      | No server listening → timeout error                                                    |
| 6   | `shutdown_reaps_both_processes`          | Spawn two dummy long-running processes, call shutdown, verify they exited              |

Tests 3-5 can use `tokio::net::TcpListener` + a small mock responder in-process (lighter than wiremock for simple health checks).

---

## 2.9 — Files Summary

| File                            | Action | What changes                                                                                    |
| ------------------------------- | ------ | ----------------------------------------------------------------------------------------------- |
| `src/executive/peripherals.rs`  | Modify | Add `llama_chat`, `llama_embed` fields, `ensure_llama_servers`, shutdown logic, probe functions |
| `src/executive/chat_session.rs` | Modify | Branch peripheral startup on backend                                                            |
| `src/config.rs` (maybe)         | Modify | Add `ready_timeout_secs` to `LlamaCppConfig`                                                    |

---

## 2.10 — Acceptance Criteria

- [ ] `eris chat` with llama.cpp backend spawns both servers, waits for readiness, responds to chat
- [ ] Exiting `eris chat` cleanly kills both llama-server processes (verify with `ps aux | grep llama`)
- [ ] If either server fails to start, clear error message with port and model path
- [ ] If llama-server is already running externally, Eris detects it and skips spawn
- [ ] Qdrant still starts regardless of backend
- [ ] Ollama path completely unaffected
- [ ] No zombie processes after Eris exit (SIGTERM + SIGKILL fallback works)
