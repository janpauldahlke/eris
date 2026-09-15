# Leif Erikson — Handover Plan (new, standalone repo)

`leif-erikson` is a self-contained, air-gapped code-execution sandbox crate,
consumed by (but independent of) the eris agent. It manages a hardened container
and runs code in it; it knows nothing about the agent that drives it.

**Status:** Not started — this is a bootstrap handover for a fresh agent instance.
**Repo:** brand-new, empty (`leif-erikson`). **Not** part of the eris repo.
**Consumed by:** eris, as an ordinary cargo dependency (git or published).
**Sibling doc:** eris-side integration lives in `AUTONOMOUS_SANDBOX_CODING.md`
(that one stays in eris; this one seeds the new repo).

---

## 0. The one rule that matters: no eris concerns

`leif-erikson` is **pure substrate**. It manages a hardened container and runs
code in it. It knows **nothing** about the agent that drives it.

**MUST NOT appear anywhere in this crate** (these belong to eris, on the far side
of the boundary):
- `FcpError` / eris error taxonomy → leif-erikson has its own `SandboxError`.
- The `Tool` trait, gatekeeper, GBNF, tool schemas → eris wraps our API in tools.
- `plan:*`, focus-loop, `AgentState`, vault, `10_Topology`, memory/Qdrant.
- Config-file parsing (figment/TOML/env) → we take a plain `SandboxConfig` struct;
  **eris owns the config source of truth** and passes it in.
- Any string like "eris", "vault", "gatekeeper", "FCP" in the public API.

**What we DO expose:** a `SandboxRunner` trait + a `bollard` implementation + plain
data types + our own error. That's the entire contract.

If you ever feel tempted to `use eris::...` — stop. The dependency arrow points
one way only: **eris → leif-erikson**, never back.

---

## 1. Mission / scope

Provide a safe, reproducible place to **write files, run commands, and iterate on
code** in Python, with two network postures:
- `Off` — fully air-gapped (no network at all).
- `PypiProxy` — the container's *only* reachable peer is a package-index proxy
  (devpi pull-through); no other egress.

Everything else (which model drives it, why, what the code is for) is out of scope.

---

## 2. Engineering laws (mirror, self-contained)

These match the parent project's discipline and are non-negotiable here too:
- **No `unsafe`.** `#![forbid(unsafe_code)]` at crate root. (bollard is pure-Rust,
  so this holds.)
- **No `unwrap()`/`expect()`** in library code — only inside `#[test]`. Everything
  routes through `SandboxError` via `?`. Deny `clippy::unwrap_used` for non-test.
- **Never block the async runtime.** All Docker/exec I/O is async (bollard is
  async); no synchronous blocking calls on the tokio executor.
- **Tests that touch the FS use `tempfile`.** Tests that need Docker are
  `#[ignore]`d by default and run in a docker-capable lane only.
- **No `Arc<Mutex<T>>` for shared mutable state across tasks** — prefer ownership +
  message passing where concurrency is needed. (A short-lived internal `Mutex` for
  a cache is acceptable; long-lived shared orchestration state is not.)

---

## 3. Public API — the contract eris depends on

Target surface (names can be refined, shape should not). All types are plain data
with `#[derive(Debug, Clone)]` and (recommended) `serde` derives so consumers can
serialize results easily.

```rust
#[async_trait::async_trait]
pub trait SandboxRunner: Send + Sync {
    /// Run a command in /scratch; enforces the per-exec wall-clock timeout and
    /// output byte cap from SandboxConfig.
    async fn exec(&self, req: ExecRequest) -> Result<ExecOutput, SandboxError>;

    async fn write_file(&self, path: &str, contents: &[u8]) -> Result<(), SandboxError>;
    async fn append_file(&self, path: &str, contents: &[u8]) -> Result<(), SandboxError>;
    /// Anchored string replace (unique `old` -> `new`); NOT line ranges.
    async fn edit_file(&self, path: &str, old: &str, new: &str) -> Result<(), SandboxError>;
    async fn read_file(&self, path: &str, range: Option<ByteRange>) -> Result<Vec<u8>, SandboxError>;
    async fn list(&self, path: Option<&str>) -> Result<Vec<DirEntry>, SandboxError>;
    async fn grep(&self, pattern: &str, path: Option<&str>) -> Result<Vec<GrepHit>, SandboxError>;

    /// Install a package. Behavior depends on SandboxConfig.network:
    ///  - PypiProxy: `uv pip install <pkg>` via the proxy index.
    ///  - Off: `uv pip install --offline <pkg>` from the pre-seeded cache/wheelhouse.
    async fn install(&self, package: &str) -> Result<ExecOutput, SandboxError>;
}

pub struct ExecRequest {
    pub command: String,
    pub args: Vec<String>,
    pub stdin: Option<Vec<u8>>,
    pub timeout: Option<std::time::Duration>, // overrides config default if set
}

pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i64,
    pub timed_out: bool,
    pub truncated: bool, // stdout/stderr hit max_output_bytes
}

pub struct SandboxConfig {
    pub network: NetworkMode,
    pub image: String,            // pinned by digest
    pub mem_limit_bytes: u64,
    pub cpu_quota: i64,           // or nano_cpus; pick one, document it
    pub pids_limit: i64,
    pub exec_timeout: std::time::Duration,
    pub max_concurrent_execs: usize,
    pub scratch_size_mb: u64,
    pub max_output_bytes: usize,
}

pub enum NetworkMode {
    Off,
    PypiProxy { index_url: String }, // the proxy the container may reach
}

pub struct ByteRange { pub start: usize, pub end: Option<usize> }
pub struct DirEntry { pub name: String, pub is_dir: bool, pub size: u64 }
pub struct GrepHit { pub path: String, pub line: u64, pub text: String }

#[derive(thiserror::Error, Debug)]
pub enum SandboxError {
    #[error("docker error: {0}")]
    Docker(String),
    #[error("exec timed out after {0:?}")]
    Timeout(std::time::Duration),
    #[error("path not allowed: {0}")]
    PathViolation(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("not ready: {0}")]
    NotReady(String),
}
```

Lifecycle (concrete impl, not on the trait):

```rust
pub struct BollardSandbox { /* container id, docker handle, semaphore, ... */ }

impl BollardSandbox {
    /// Lazily create + start the hardened container (and, if PypiProxy, ensure the
    /// proxy + internal network exist). Idempotent per instance.
    pub async fn start(config: SandboxConfig) -> Result<Self, SandboxError>;
    /// Stop + remove the container (and proxy/network if we own them).
    pub async fn shutdown(self) -> Result<(), SandboxError>;
}
// impl SandboxRunner for BollardSandbox { ... }
```

**eris' side (for context only — do NOT build it here):** eris implements one
`impl Tool` per operation, calls these methods, and maps `SandboxError -> FcpError`.

---

## 4. Internal architecture

- **bollard** for all Docker ops. `BollardSandbox` owns the container id + a
  `tokio::sync::Semaphore` sized to `max_concurrent_execs`.
- **Container spec (hardening):**
  - `NetworkMode`: `"none"` for `Off`; the internal proxy network name for
    `PypiProxy`.
  - `ReadonlyRootfs: true`; **tmpfs** mount at `/scratch` sized to `scratch_size_mb`.
  - non-root `User`; `CapDrop: ["ALL"]`; `SecurityOpt: ["no-new-privileges"]` +
    default seccomp.
  - `Memory`, `NanoCpus`/`CpuQuota`, `PidsLimit` from config.
  - working dir `/scratch`; no host bind-mounts.
- **exec:** `create_exec` + `start_exec`, wrapped in `tokio::time::timeout`
  (`ExecRequest.timeout` or config default). On timeout → kill the exec/PID, set
  `timed_out = true`. Cap collected stdout/stderr at `max_output_bytes`
  (`truncated = true`).
- **file ops:** prefer bollard `upload_to_container` / `download_from_container`
  (tar streams) for `write/append/read`; implement `edit_file` as read → anchored
  replace → write; `list`/`grep` via `exec` (`ls`, `grep -rn`).
- **network=PypiProxy setup:** create an `internal: true` docker network; run a
  **devpi** container attached to both that internal net and an outbound net; set
  the sandbox's `UV_INDEX_URL`/`PIP_INDEX_URL` to the proxy. The sandbox has no
  other route out. (Whether leif-erikson *manages* the proxy or expects one to be
  provided via `index_url` is a design choice — document whichever you pick.)

---

## 5. Phase 1 deliverables (naked repo → working exec loop)

1. `cargo init --lib`; crate name `leif-erikson` (lib `leif_erikson`).
2. `Cargo.toml` deps: `bollard`, `tokio` (rt-multi-thread, macros, process, time,
   io-util), `async-trait`, `thiserror`, `tracing`, `futures`; optional `serde`
   (derive) on public types; dev: `tempfile`. Crate root: `#![forbid(unsafe_code)]`
   + `#![deny(clippy::unwrap_used)]` (allow in test).
3. Define the public API from §3 (`SandboxRunner`, types, `SandboxError`).
4. Implement `BollardSandbox::{start, shutdown}` with the hardened spec (§4),
   `network=Off` first.
5. Implement `exec` with timeout + output cap; then `write/append/read/edit/list/grep`.
6. Add `network=PypiProxy` + `install` (uv).
7. Tests:
   - Unit: a `MockSandbox` implementing `SandboxRunner` for consumer-style tests;
     pure logic (anchored `edit_file`, range math, truncation) tested directly.
   - Integration: `#[ignore]`d tests that actually start a container (write → exec
     `python -c` → read stdout; timeout is enforced; readonly rootfs blocks writes
     outside `/scratch`). Run in a docker lane.
8. README stating the mission, the boundary rule (§0), and the two network modes.
9. Minimal CI: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`
   (Docker integration lane optional/self-hosted).

**Definition of done for Phase 1:** from a test, `BollardSandbox::start(Off)` →
`write_file("hello.py", ...)` → `exec("python", ["hello.py"])` returns the expected
stdout with `exit_code == 0`, and a deliberately slow command trips `timed_out`.

---

## 6. Versioning & consumption by eris

- Tag semver releases; eris pins a version (git rev or published version).
- During co-development, eris uses a `[patch]` / path override to a local clone.
- Breaking API changes = major bump; keep the `SandboxRunner` surface stable.
- **No git submodule** in eris (agent footgun). Plain cargo dependency only.

---

## 7. Out of scope (lives in eris, not here)

- Tool wrapping, gatekeeper allowlists, GBNF, descriptors, skills.
- `plan:*` ledger, focus-loop, operator hold/resume, model exclusivity.
- Anything about vaults, memory, or the agent's cognition.
- Config file formats — we only accept `SandboxConfig`.

If a feature request implies knowing *why* code runs or *who* runs it, it belongs
in eris. leif-erikson only knows *how* to run it safely.
