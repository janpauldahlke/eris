# Tools & Gatekeeper

The tool system in Eris is strictly gated and semantically routed. To avoid blowing up the LLM's context window with dozens of JSON schemas, Eris employs a `Gatekeeper` and a `ToolRouter`.

## Tool Routing Mechanism

Instead of sending every available tool schema to the LLM on every turn:
1. The user's input is passed to the `ToolRouter`.
2. The `ToolRouter` generates an embedding of the user's input.
3. It computes the cosine similarity against pre-computed embeddings of tool descriptions.
4. Tools scoring above the `threshold` (or forcibly included via lexical heuristics, like detecting a URL) are selected as **Tier 1**.
5. **Tier 1** tools have their full JSON schemas injected into the system prompt.
6. The remaining tools (**Tier 2**) only have their names and brief descriptions injected, informing the LLM that they exist without paying the token cost of their full arguments schema.

## The Gatekeeper

The `Gatekeeper` (`src/tools/gatekeeper.rs`) manages registration and execution.

### Tool Registration
During boot (`execute_command` in `router.rs`), tools are instantiated and registered with the Gatekeeper. Registration requires wrapping the tool in an `Arc` that implements the `Tool` trait.

### Embedded Descriptors (JIT Guidance)
To further guide the LLM, Eris uses `ToolDescriptorRegistry`. These are compile-time embedded hints (When to use, When not to use, Good/Bad examples) that are conditionally injected as JIT (Just-In-Time) guidance *only* for the Tier 1 tools selected for that specific turn.

## Available Tool Categories

- **Vault Operations**: 
  - `vault:read`: Read content from local markdown files.
  - `vault:write`: Append or create markdown files in the workspace.
  - `vault:list`: List files in the workspace.
- **Memory Operations**:
  - `memory:stage`: Temporarily hold data in Ephemeral Memory.
  - `memory:staged_list`: View pending staged items.
  - `memory:commit`: Move a staged item to Semantic Brain and write it to the Vault.
  - `memory:commit_all`: Bulk commit all pending items.
  - `memory:query`: Search the Semantic Brain (Qdrant).
- **Agenda/Tasks**:
  - `agenda:push`: Add a task to the local `.fcp_agenda.json`.
  - `agenda:list`: View pending tasks.
  - `agenda:complete`: Mark a task as done.
- **Web**:
  - `web:fetch`: Scrape a URL and store the chunked content in Ephemeral memory (and index in Qdrant).
  - `web:artifact_query`: Search a previously fetched web artifact for specific keywords.
- **Clock & System**:
  - `clock:now`: Get current wall-clock time.
  - `clock:timer`: Set a relative countdown timer.
  - `clock:wall_alarm`: Set an absolute alarm (e.g., "wake me at 7 AM").
  - `system:health`: Retrieve basic CPU/Memory telemetry.

## Execution Flow

When the Orchestrator receives an `ExecuteTools` directive:
1. It validates the tool name against the Gatekeeper's allowed list.
2. It generates a SHA-256 fingerprint of the tool name and normalized JSON arguments.
3. If an identical fingerprint was executed successfully in the exact same batch/turn, it is suppressed (Duplicate suppression).
4. The tool is executed asynchronously.
5. Successes append the tool's stringified output to the chat stack.
6. Failures trigger either a `TargetedSchemaRetry` (if parsing/schema failed) or a `Recover` state to inform the LLM of the system error.