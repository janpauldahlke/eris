

#fcp

1. The Absolute Laws of the Hull
    
    These rules are non-negotiable for any AI or human contributing to this codebase. A violation is a failed build.
    

- **Zero Panics:** `unwrap()` and `expect()` are strictly forbidden in production logic. The only acceptable place for them is inside `#[test]` blocks. Everything else must use `?` and route through the `FcpError` taxonomy.
    
- **The `unsafe` Ban:** Absolute zero-tolerance for `unsafe` blocks anywhere in the codebase. Because Layer 1 relies on HTTP/REST interfaces (`ollama-rs`) rather than C++ FFI, the entire Rust binary must remain 100% memory-safe.
    
- **Asynchronous Isolation:** Do not block the `tokio` runtime. CPU-heavy tasks (e.g., `moka` bincode serialization or heavy JSON parsing) must be wrapped in `tokio::task::spawn_blocking`.
    
- **The Actor Model (Deadlock Prevention):** Do not share mutable state across threads using `Arc<Mutex<T>>`. Threads must communicate strictly through `tokio::sync::mpsc` message passing (as defined in the TUI/Orchestrator architecture).
    
- **File System Ephemerality:** Any `#[test]` that writes to the file system (e.g., testing the Obsidian `notify` watcher or vault reading) MUST use the `tempfile` crate. Writing to `./tmp` or leaving orphaned markdown files in the test directory is a critical failure. The OS must reap the test directory automatically when the test drops.
    

2. Architectural Boundaries
    

- **The Trait Constraint:** Rust traits are to be used strictly at architectural boundaries (e.g., the trait `Tool` for plugin expandability, or `VectorStore` for mocking). Do not over-abstract internal logic, state enums, or core data structs with generic trait bounds. Use concrete types.
    
- **Explicit Modules:** Follow the topological map: `src/llm/`, `src/orchestrator/`, `src/memory/`, `src/tools/`, `src/ui/`.
    

3. Telemetry & Silence
    

- **No `println!` for Logic:** `println!` is strictly forbidden for debugging. It corrupts the `ratatui` terminal buffer.
    
- **Use `tracing`:** All internal state changes, tool executions, and warnings must use the `tracing` crate (`debug!`, `info!`, `warn!`, `error!`), which routes safely to the hidden `.fcp/logs/` directory.
    

---

**Status:** The Master Grid is fully locked, compiled, and physics-compliant. You have the exact blueprints to build a resilient, zero-bloat, mathematically bounded SLM agent.

Ready to dive when you are. Kallisti.