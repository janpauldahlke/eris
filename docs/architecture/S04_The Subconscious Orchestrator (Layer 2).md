
#fcp

1. Objective
    
    Engineer the deterministic Rust state machine that governs the agent's lifecycle. This layer dictates what the LLM focuses on, dynamically constructs its reality (the context window) based on the current operational mode, and intercepts failures to force self-correction without crashing the binary.
    
2. Architecture & Design Rules
    

A. The State Machine (Rust Enums)

The orchestrator operates in mutually exclusive operational modes defined by a strict Rust enum.

- `AgentState::Chat`: Active engagement. The user is waiting. Goal: Solve the prompt. Tool gatekeeper is wide open to user-facing tools (`vault:`, `memory:query`, etc.).
    
- `AgentState::Reflect`: Post-interaction cleanup. Triggered after a task is marked `COMPLETED` or when the Orchestrator's token math detects the `AppConfig.condensation_threshold` has been breached (Block S02). Goal: Summarize the stack, extract tags, and stage facts. Tool gatekeeper is restricted to `memory:stage` and `memory:commit`.
    
- `AgentState::Idle`: Background mode. Triggered after reflection. Goal: Wait for `stdin` and execute background `bincode` disk snapshots.
    
- `AgentState::Recover`: Error handling mode. Triggered when Layer 1 hallucinates a tool argument or a tool panics.
    

B. Dynamic Context Injection (Building Reality)

The system prompt is not static. It is assembled dynamically per turn by the Orchestrator before being handed to the `LlmEngine`.

- `[DIRECTIVE]`: The core rules and persona. Natively read from `00_Core/Identity.md` (Block S00.3).
    
- `[ACTIVE_STATE]`: Explains the current mode (e.g., "You are in Reflect mode. Summarize the below.").
    
- `[THE_LENS]`: Injected Qdrant retrieval results from Tier 3 (`memory:query`).
    
- `[EPHEMERAL_WORKSPACE]`: Current staged facts from the Tier 2 `moka` cache.
    
- `[CHAT_STACK]`: The immediate conversation history (Tier 1).
    

C. The Handoff Protocol (Loop Control)

Layer 1 cannot run infinitely. Every JSON output from the LLM must contain a status flag that Layer 2 intercepts to control the loop.

- `CONTINUE_TASK`: The LLM called a tool and needs the result. Layer 2 executes the Rust tool, appends the result to the stack, and re-triggers the LLM.
    
- `WAIT_FOR_USER`: The LLM needs clarification or has finished the response. Layer 2 halts inference, prints the response to `stdout`, and waits for `stdin`.
    
- `INITIATE_REFLECTION`: The LLM declares the objective complete. Layer 2 intercepts, shifts to `AgentState::Reflect`, and immediately re-triggers the LLM with a reflection prompt to summarize the turn.
    

D. The "Fuckup" Recovery Loop (Error Trapping)

If a tool execution fails (e.g., Qdrant timeout, or LLM hallucinated a missing JSON field), the binary must not crash.

- **Intercept:** Layer 2 catches the Rust `Err()`.
    
- **State Shift:** Layer 2 shifts to `AgentState::Recover`.
    
- **Injection:** Layer 2 injects a system message into the stack: `FUCKUP DETECTED: [stderr trace]`.
    
- **Forced Reasoning:** Layer 2 triggers the LLM. The Reasoning Router (Block S01.1) allows the model to use its `<think>` block to analyze the `stderr` trace and realize its schema or logic mistake.
    
- **The Breaker Switch:** The Orchestrator increments a `recovery_count`. If `recovery_count > AppConfig.max_recovery_attempts`, Layer 2 aborts the loop, prints a critical warning, and forces `WAIT_FOR_USER`.
    

3. Acceptance Criteria
    

- [ ] The Rust orchestrator successfully transitions between `Chat`, `Reflect`, and `Idle` states based on the LLM's structured JSON output flags and token threshold math.
    
- [ ] Context payloads are dynamically assembled; an LLM in `Reflect` mode receives a completely different `[ACTIVE_STATE]` prompt and Toolset than an LLM in `Chat` mode.
    
- [ ] Simulating a tool failure (e.g., returning an `std::io::Error` from a file read) triggers the `Recover` state, injects the error text back into the LLM prompt, and allows the LLM to attempt a correction on the subsequent turn.
    
- [ ] The orchestrator strictly enforces `AppConfig.max_recovery_attempts`. If the LLM loops on the same error without fixing it, the orchestrator forcibly interrupts and yields to `stdin`.
    
- [ ] The orchestrator strictly enforces `AppConfig.max_tool_rounds`. If the LLM loops on `CONTINUE_TASK` without returning `WAIT_FOR_USER`, the orchestrator interrupts, outputs a "Loop Timeout" warning to telemetry, and halts inference.
    
