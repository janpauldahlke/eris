
#fcp

1. Objective
    
    Enforce a strict "Context Diet" and physical security perimeter for the LLM's toolset. Providing excessive tools to a 9B/32B model guarantees attention dilution and hallucination. The Gatekeeper dynamically filters the ToolRegistry based on the active `AgentState`, translating allowed tools into Ollama's native API schema. Crucially, it acts as the absolute firewall against path traversal and schema violations.
    
2. Architecture & Design Rules
    

A. The Tool Registry (The Arsenal)

All available capabilities are registered at startup in a centralized, read-only Rust `HashMap`.

- **Tool Definition (`ToolDef`):** Every tool implements a standard asynchronous trait containing:
    
    - `name`: A strict `domain:action` identifier (e.g., `memory:query`, `vault:write`).
        
    - `description`: A highly dense, token-optimized explanation of when to use it.
        
    - `input_schema`: A standard JSON Schema object defining required parameters, derived automatically via the Rust `schemars` crate.
        
    - `domain`: An enum categorizing the tool (`Vault`, `Memory`, `System`, `Sensory`).
        
    - `handler`: The async execution logic.
        

B. Context-Adaptive Gating (The Filter)

Before Layer 2 hands control to the HTTP Engine, the Gatekeeper builds a `ToolSet` specifically for that exact turn and formats it into the Ollama `"tools"` JSON array.

- **Mode-Based Defaults:**
    
    - `AgentState::Reflect`: Strictly restricted to Tier 2 memory operations (`memory:stage`, `memory:commit`).
        
    - `AgentState::Idle`: Restricted to background tasks (`memory:commit`, `vault:read`).
        
    - `AgentState::Chat`: Access to active user-facing tools (`memory:query`, `vault:read`, `vault:write`).
        
- **Conditional Injection (Dynamic Visibility):** Tools are physically withheld from the API payload unless prerequisites are met. For example, if the Gatekeeper checks the `EphemeralStore` (Block S03.1) and the `moka` cache is empty, the `memory:commit` tool is completely stripped from the context. The LLM cannot hallucinate a commit operation if it doesn't know the tool exists.
    

C. The Execution Router & The Firewall

When the LLM yields a tool call via the Ollama response, the Gatekeeper acts as the final firewall before executing the Rust logic.

- **Schema Validation:** It strictly validates the LLM's JSON arguments against the `schemars` definition. If required fields are missing, it instantly blocks execution and yields `Err(FcpError::SchemaViolation)`.
    
- **The Path Traversal Firewall:** If the tool is `vault:read` or `vault:write`, the Gatekeeper intercepts the `file_path` argument.
    
    - If the path contains `../` or attempts to escape `vaults/{workspace}/`, it yields `Err(FcpError::ToolFault("Security Violation: Path Traversal Denied"))`.
        
    - If a `vault:write` targets `00_Core/`, it yields `Err(FcpError::ToolFault("Security Violation: 00_Core is Immutable"))`.
        
- **Redirection:** All intercepted errors are passed to the Orchestrator, triggering the `Recover` state (Block S04.2) to feed the strict error string back to the LLM for forced correction.
    

D. Direct Tool Dispatch (CLI Override)

To facilitate rapid debugging without spinning up the Ollama daemon or loading VRAM, the Gatekeeper exposes a direct execution route via the CLI.

- **Command:** `fcp tool <name> <json_args>`
    
- **Execution:** Parses the JSON from `stdin`, locates the tool by its `NOUN:VERB` string in the Registry, executes the native Rust handler against the current workspace, and prints the raw result to `stdout`.
    

3. Acceptance Criteria

- [ ] The Gatekeeper successfully generates a strictly formatted JSON array matching Ollama's native tool schema, containing only the allowed tools for a given `AgentState`.
- [ ] If the active state is `Reflect` and the `moka` cache count is 0, the `memory:commit` tool schema is entirely absent from the payload sent to the Ollama Engine.
- [ ] The Gatekeeper successfully catches an LLM JSON output that attempts to use an unauthorized tool or is missing a required argument, returning a structured `FcpError::SchemaViolation`.
- [ ] The Path Firewall successfully intercepts a `vault:write` attempting to target `../system_file` or `00_Core/Identity.md`, aborting the write and returning a `ToolFault`.
- [ ] Running `fcp tool memory:query '{"query": "test", "filter_tag": "procedural"}'` directly executes the native Rust function and prints the result to the terminal without loading the LLM weights.

