

#fcp

1. Objective
    
    Manage the highly volatile layers of the agent's memory. Tier 1 (The Stack) prevents the context window from collapsing via dynamic condensation. Tier 2 (The Ephemeral) acts as a high-speed, bounded, in-memory staging ground for extracted facts, featuring automatic garbage collection and workspace-isolated disk persistence across terminal sessions.
    
2. Architecture & Design Rules
    

A. Tier 1: The Stack (Reactionary Condensation)

The Orchestrator relies strictly on the HTTP response metrics from the `LlmEngine` for token math, as Ollama does not natively warn before context exhaustion.

- The Trigger: The Orchestrator extracts `prompt_eval_count` + `eval_count` from every inference response. If `total_tokens >= (AppConfig.num_ctx * AppConfig.condensation_threshold)`, Layer 2 halts standard interaction.
    
- The Condensation Loop:
    
    1. The Orchestrator forces a transition to the `AgentState::Reflect` state.
        
    2. It injects a system-level command into the active `[CHAT_STACK]`: _"System Directive: Summarize the current conversation into `AppConfig.condensation_target` tokens of core facts, active decisions, and open questions. Output strictly as JSON."_ 3. The LLM generates the summary.
        
    3. The Orchestrator physically flushes the raw chat history from its internal state and replaces the array entirely with the single `session:running_summary` string.
        

B. Tier 2: The Ephemeral (In-Memory TTL)

The graveyard of Redis. We manage volatile state directly inside the Rust process heap to eliminate IPC latency.

- Tooling: `moka::future::Cache` (Concurrent, async-aware cache with native TTL).
    
- Key Structure: Use hierarchical string keys:
    
    - `session:running_summary` -> Overwritten per condensation cycle.
        
    - `promote:{timestamp}.{tag}` -> Holds extracted facts. TTL set by `AppConfig.ephemeral_ttl_secs` (e.g., 7200 seconds / 2 hours).
        
- The Attention Filter: If a `promote` key expires in the `moka` cache before the LLM explicitly calls the `memory:commit` tool (Tier 3), it evaporates. It was mathematically deemed not important enough to vectorize.
    

C. Persistence & Garbage Collection (The Overflow Guard)

We ensure the Ephemeral tier never breaches memory bounds or bloats the hard drive.

- RAM Bounds: The `moka` cache is initialized with a hard `max_capacity` (e.g., 10,000 entries). If the LLM spams the cache, `moka` automatically evicts the oldest entries to prevent an OS panic.
    
- Disk Snapshots (Write): A background `tokio` task wakes every `AppConfig.snapshot_interval_secs` (and immediately upon receiving the `CancellationToken` during SIGINT). It drops natively expired keys, serializes the active data using `bincode`, and writes to `.fcp/ephemeral_{workspace}.bin` via a File Truncate operation. We never append.
    
- Boot Ingestion (Read): On startup, `fcp` deserializes `.fcp/ephemeral_{workspace}.bin`. It performs a secondary time-check against absolute system time: if the system was offline longer than the TTL, the keys are discarded. Survivors are injected back into `moka`.
    

3. Acceptance Criteria
    

- [ ] Tracking `eval_count` past the 75% threshold successfully triggers an automatic `Reflect` cycle on the subsequent turn, dynamically shrinking the context payload to a single summary string.
    
- [ ] The `moka` cache successfully auto-evicts keys once their TTL expires or the `max_capacity` boundary is breached.
    
- [ ] Sending SIGINT triggers the `CancellationToken`, forcing the snapshot task to serialize the unexpired cache to the correct workspace-isolated `.bin` file before the process exits.
    
- [ ] Restarting the binary after the TTL period results in an empty `moka` cache, proving the boot-loader correctly honors absolute timestamps from the deserialized file.
    
