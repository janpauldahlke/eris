

#fcp

1. Objective
    
    Establish an indestructible, panic-free execution environment. This block provides the CLI routing, the domain-specific error hierarchy, and the introspection "Black Box" required to debug the state machine and memory systems later. The core philosophy is absolute resilience: the orchestrator must never crash unexpectedly.
    
2. Architecture & Design Rules
    
    A. The Executive (CLI Scaffolding)
    
    The system operates as a pure, single-binary CLI. It routes user intent to the orchestrator.
    

- Tooling: Must use `clap` (derive API).
    
- Global Arguments:
    
    - `--vault` / `-v` (Environment override: `FCP_VAULT`): Path to the Obsidian memory directory. Default: `./obsidian-vault`.
        
    - `--verbose` / `-V`: Stackable flag to increase telemetry verbosity (`INFO` -> `DEBUG` -> `TRACE`).
        
- Execution Modes (Subcommands):
    
    - `chat`: Boots the interactive State Machine loop and the TUI.
        
    - `run <prompt>`: Executes a single-shot agentic turn and outputs raw to stdout (headless mode).
        
    - `tool <name> [args...]`: Bypasses the LLM entirely to fire a specific tool function (critical for isolated tool testing).
        

B. The Mavis Stricture (Zero-Panic Error Handling)

Errors are data, not anomalies. The orchestrator must react to them, not die from them.

- Tooling: `thiserror` for structured, type-safe error variants.
    
- The Law: Zero `unwrap()`, zero `expect()`, absolute zero `unsafe` (enforced at the crate root).
    
- Error Domains: The global `FcpError` enum must categorize failures distinctly:
    
    - `Io`: File system, vault access, or cache read/write failures.
        
    - `Config`: Missing environment variables or invalid CLI args.
        
    - `Engine`: Network faults reaching the Ollama daemon, generation timeouts, or catastrophic JSON deserialization failures from the model's output.
        
    - `Orchestration`: State machine logic errors or LLM context budget collapse.
        
    - `Tool`: Execution failure within a gated tool (routed back to the LLM via the Fuckup Loop).
        
- Routing: Every fallible function must return `Result<T, FcpError>`.
    

C. The Black Box (Telemetry)

Introspection must be highly structured to track where the LLM's attention or network latency breaks down.

- Tooling: `tracing` and `tracing-subscriber`.
    
- I/O Separation: All telemetry, warnings, and error traces must strictly output to `stderr` (or the `.fcp/logs/` file in UI mode). `stdout` is exclusively reserved for the final parsed output of the agent or direct tool results (allowing the headless binary to be piped in shell scripts).
    
- Span Hierarchy: The logger must support nested spans to isolate bottlenecks:
    
    - `Turn Span`: Tracks total time and aggregate token usage per user interaction.
        
    - `Gatekeeper Span`: Tracks tool selection math and schema injection.
        
    - `Inference Span`: Tracks HTTP latency to the `LlmEngine`, Time To First Token (TTFT), and the final `eval_count` (tokens per second) returned by the Ollama API metrics.
        

3. Acceptance Criteria
    

- [ ] The codebase compiles cleanly with `#![deny(clippy::unwrap_used)]` and `#![forbid(unsafe_code)]` enforced at the crate root.
    
- [ ] Running `fcp --help` outputs the correctly structured command tree.
    
- [ ] Running `fcp tool non_existent_tool` fails gracefully, prints a structured `FcpError::Config` to `stderr`, and exits with a non-zero status code (no Rust panic trace).
    
- [ ] Setting `RUST_LOG=debug fcp run "test"` emits structured formatted logs to `stderr` detailing the boot sequence, while isolating the final output to `stdout`.
    
