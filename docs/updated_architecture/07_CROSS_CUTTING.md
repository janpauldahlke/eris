# Cross-cutting concerns

## Error taxonomy (`executive/error.rs`)

`FcpError` variants: `Io`, `Config`, `WorkspaceFault`, `EngineFault`, `NetworkFault`, `ContextExhaustion`, `SchemaViolation`, `Cancellation`, `ToolFault`, `ParseFault`, `VectorDbOffline`, `EmbeddingFault`, `Interrupted`.

Orchestrator maps many failures into recovery loops or idle; **`Interrupted`** is special (heartbeat idle injection).

## Async and concurrency

- **Tokio** runtime (`#[tokio::main]`).
- **No `Arc<Mutex<>>`** for orchestrator state—single task owns `Orchestrator`; UI talks via channels.
- **Cross-task flags:** a shared `Arc<AtomicBool>` (`promotion_suppressed_during_step`) coordinates the snapshot daemon with `Orchestrator::step` using `SeqCst` load/store only—no mutex on orchestrator state.
- **CPU-heavy work:** bincode in ephemeral, some JSON work uses `spawn_blocking`.
- **`CancellationToken`** (tokio-util) for graceful shutdown.

## Context window / condensation (`orchestrator/context/window.rs`, re-exported as `orchestrator::context`)

When estimated tokens exceed threshold, older messages fold into a **rolling summary** stored as ephemeral entry `fcp:rolling_context_summary` with structured JSON (`RollingSummaryV1`). Web-heavy turns may use a higher condensation threshold (`WEB_CONDENSATION_THRESHOLD` in core).

## Security posture (code-level)

- Tools validate paths (`validation`) for writes.
- Gatekeeper schema validation on all tool args.
- No `unsafe` in crate.

## Workspace rules (reminder for agents)

From `.cursorrules`:

- No `unwrap`/`expect` outside tests.
- No `unsafe`.
- No blocking on tokio for heavy CPU—use `spawn_blocking`.
- Tests that write FS must use `tempfile`.
- Use `tracing!` not `println!` for logic.
- Do not commit git on behalf of the user.

## Tests

Distributed across modules: `#[cfg(test)]` and `#[tokio::test]`. Integration-style tests in `router.rs` for orchestrator ordering; Ollama tests use wiremock.
