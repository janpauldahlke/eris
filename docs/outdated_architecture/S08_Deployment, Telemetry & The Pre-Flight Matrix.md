

#fcp

1. Objective
    
    Establish the physical deployment profile and the post-mortem telemetry architecture. Because the TUI (Block S07) aggressively hijacks the terminal buffer, standard `println!` debugging is rendered useless. This block defines how the Orchestrator safely logs its internal state without polluting its own memory, how it strictly verifies its external Ollama/Qdrant dependencies before booting, and the compilation physics required for maximum efficiency.
    
2. Architecture & Design Rules
    

A. The Black Box (Persistent, Rotated Tracing)

The Orchestrator must maintain a rolling, time-stamped log of its internal monologue and error states, physically separated from the LLM's sensory watcher.

- **Tooling:** `tracing` and `tracing-appender` crates.
    
- **Execution (Anti-Pollution):** The logs are written strictly to `.fcp/logs/` (the hidden system directory). This guarantees the Obsidian file watcher (Block S03) ignores them, preventing an infinite log-ingestion loop.
    
- **Log Rotation:** Layer 2 instantiates `tracing_appender::rolling::daily`. This automatically rotates the log at midnight, creating files like `fcp_core.2026-03-27.log` to prevent infinite disk bloat.
    
- **Log Levels:**
    
    - `TRACE`: The raw LLM token stream and JSON schema parsing steps.
        
    - `DEBUG`: State machine shifts (`Chat` -> `Reflect`), cache evictions.
        
    - `INFO`: Tool execution summaries and Vault I/O operations.
        
    - `WARN`: LLM JSON hallucinations (Cognitive Faults routed to the Fuckup Loop).
        
    - `ERROR`: System Fatalities (Daemons offline, missing directories).
        

B. The Pre-Flight Matrix (Sidecar Verification)

The Rust binary relies on two external daemons. Before the TUI initializes, the Orchestrator performs a blocking verification sequence. If any step fails, the binary aborts with a raw `stderr` print containing the exact remediation command.

1. **The Qdrant Ping:** Attempts an HTTP/gRPC connection to `localhost:6334`.
    
    - _Failure Output:_ `FATAL: Qdrant sidecar not detected. Run 'docker compose up -d' in the fcp root.`
        
2. **The Ollama Ping:** Attempts an HTTP GET to `localhost:11434/api/tags`.
    
    - _Failure Output:_ `FATAL: Ollama daemon not responding. Ensure Ollama is running.`
        
3. **The Weights Check:** Parses the `/api/tags` JSON response to verify both `AppConfig.model_name` (e.g., `qwen2.5:32b`) and `nomic-embed-text` are physically present on the disk.
    
    - _Failure Output:_ `FATAL: Missing required models. Run 'ollama pull nomic-embed-text' and 'ollama pull {model_name}'.`
        

C. Build Physics (The Release Profile)

To handle heavy async workloads and continuous UI rendering without dropping frames, the Rust compiler must be tuned for maximum binary optimization.

- **Cargo.toml Profile:**
    

Ini, TOML

```
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort" # Drop unwinding to save binary size and overhead
```

D. Telemetry Extraction (The CLI Bypass)

When the user needs to analyze why the LLM got stuck in a recovery loop while AFK, we need a fast extraction method that targets today's rotated log without booting the TUI.

- **Command:** `fcp log --tail 100` or `fcp log --errors`
    
- **Logic:** A CLI bypass that locates the active `.fcp/logs/fcp_core.{date}.log` file, parses it, and prints a color-coded, human-readable timeline of the state machine's recent decisions.
    

3. Acceptance Criteria
    
- [ ] Booting the binary successfully creates the `.fcp/logs/` directory and writes structured log lines to a date-stamped file without interfering with the active `ratatui` terminal buffer.
- [ ] Modifying the log file does not trigger the `notify` watcher from Block S03, proving the system is immune to log-ingestion loops.
- [ ] Booting `fcp` while the Qdrant container or Ollama daemon is stopped successfully catches the network failure during the pre-flight check, aborts gracefully, and prints the exact remediation command.
- [ ] The Pre-Flight check successfully queries Ollama's local registry to ensure both the LLM and Embedding models are pulled before allowing the binary to start.
- [ ] Compiling with `cargo build --release` successfully applies Link-Time Optimization (LTO) and outputs a stripped binary configured to abort on panic.
