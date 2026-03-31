ROJECT BRIEFING: ERIS (FCP Orchestrator) - Continuation Session

You are working on ERIS, a local SLM orchestrator written in Rust. It's a ratatui TUI that talks to Ollama (targeting qwen3.5:27b on a GPU rig, currently qwen2.5:14b on Mac) with a cognitive loop, tool system, and vector memory via Qdrant.

Architecture (read these files first to orient):

src/main.rs - Entry point, loads config, routes to execute_command
src/executive/router.rs - Wires Chat command: engine, gatekeeper, orchestrator, TUI
src/orchestrator/core.rs - THE cognitive loop (step()): assemble context → LLM generate → parse JSON response → execute tools or halt
src/orchestrator/state.rs - AgentState (Chat/Reflect/Idle/Recover), LlmResponse struct, LoopDirective enum
src/orchestrator/context.rs - Builds system prompt from Identity.md + tool schemas
src/engine/ollama.rs - Ollama HTTP client, JSON mode, streaming support
src/tools/gatekeeper.rs - Tool registry, state-based access control, JSON schema validation
src/tools/traits.rs - Tool trait boundary
src/config.rs - AppConfig with figment (fcp.toml → env → CLI)
src/memory/ephemeral.rs - Moka cache with bincode snapshot daemon
src/memory/semantic.rs - Qdrant vector search via ollama embeddings
Workspace layout: User runs from inside a workspace dir (e.g., adam/). 00_Core/Identity.md is there. .fcp_seal marks initialization. fcp.toml holds config.

What was already fixed in the previous session (don't re-fix these):

Config loading - AppConfig::load() wired into router (was using AppConfig::default())
Workspace path geometry - ContextAssembler now finds 00*Core/Identity.md correctly
Recovery matrix - Io and ParseFault from tools now trigger recovery instead of crashing orchestrator
All tools registered with gatekeeper (vault:read/write/list, agenda:push/list/complete, web:fetch, memory:commit/query)
Assistant responses pushed into chat_stack (LLM now has conversational memory)
num_ctx passed to Ollama via GenerationOptions
message_to_user extracted and displayed as [ERIS]: ... in TUI
Heartbeat monitor, snapshot daemon, and CancellationToken all wired
Comprehensive tracing added to orchestrator, engine, gatekeeper, context assembler
All expect() removed from production code, clippy clean, 69/69 tests pass, zero warnings
Double tool injection fixed (engine no longer injects empty {} tools blob)
Gatekeeper state routing expanded (memory:query in Reflect/Idle, vault:write in Idle)
vault:write no longer routes core*\* filenames to immutable 00_Core
Dead scratch files deleted, unused imports cleaned
What still needs to be done (prioritized):

P0 - System prompt engineering for qwen3.5:27b The current prompt in context.rs is verbose prose. Qwen models respond much better to:

A concrete JSON example (few-shot) rather than schema description
Short, imperative instructions
The thought field should be labeled as internal reasoning the user won't see
Status semantics need clarification (the LLM keeps outputting Reflect with empty tool_calls, which triggers recovery)
P1 - Streaming JSON parser (Fatal Flaw B) TUI freezes during generation because OllamaClient::generate() waits for complete JSON. Need a partial JSON accumulator that updates the viewport token-by-token. The streaming path exists (stream_tx parameter) but the orchestrator never uses it.

P2 - Episodic condenser (Fatal Flaw A) No protection against unbounded chat_stack growth. execute_condensation() exists but is never called. Need a check after each loop iteration: if token count > condensation_threshold, call it automatically.

P3 - Parallel tool execution When LLM returns multiple tool_calls, they execute sequentially. Use tokio::join! or futures::future::join_all for concurrent execution.

P4 - memory:stage tool is a stub Always returns Err(ToolFault). Needs actual moka cache injection via the EphemeralMemory instance.

Coding rules (from .cursorrules, non-negotiable):

Zero unwrap()/expect() in production code (only in #[test] blocks)
Zero unsafe blocks
No println! - use tracing crate only
CPU-heavy work in tokio::task::spawn_blocking
No Arc<Mutex<T>> - use tokio::sync::mpsc channels
Tests that touch filesystem MUST use tempfile crate
Work one function at a time, stop after each block for cargo test
Test command: cargo test from project root. Currently 69/69 pass.

The docs/\_POST_MORTEM.md file contains the full forensic analysis from the previous session. Read it for additional context on design decisions and remaining issues.

That's everything they need to hit the ground running without re-reading 30+ source files.

---

User sends message
→ embed user input with nomic (~50-100ms, cheap)
→ cosine similarity against tool descriptions
→ match? YES → assemble prompt WITH tool schemas (existing assemble())
→ match? NO → assemble prompt WITHOUT tools (assemble_conversational())
→ single LLM call
→ process normally
