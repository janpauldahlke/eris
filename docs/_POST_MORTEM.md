Issue List: ERIS Post-Mortem
P0 - CRASH-CAUSING BUGS (fix these first or nothing works)

1. Config is never loaded - hardcoded defaults used everywhere

executive/router.rs:38:

router.rs
Lines 38-38
let config = Arc::new(crate::config::AppConfig::default());
The ignition wizard writes fcp.toml to the workspace with model_name = "qwen2.5:14b-instruct", but the router creates AppConfig::default() which hardcodes model_name = "llama3.2". The entire figment config hierarchy (AppConfig::load()) is never called. So:

Wrong model name sent to Ollama
Wrong num_ctx, wrong qdrant_collection, wrong everything
The ignition wizard's output is decorative 2. Workspace path geometry is broken - Identity.md is never found

executive/router.rs:74-75 hardcodes workspace as "default". ContextAssembler::new(vault_root, "default") constructs core_dir as {cwd}/default/00_Core/. But ignition creates 00_Core/ directly under the workspace root (no default/ subdirectory). So:

context.rs
Lines 12-13
pub fn new(vault_root: &std::path::Path, workspace: &str) -> Self {
Self { core_dir: vault_root.join(workspace).join("00_Core") }
The assembler looks for adam/default/00_Core/Identity.md but the file is at adam/00_Core/Identity.md. The fallback kicks in every time. The agent never reads its own identity.

3. I/O errors from tools crash the orchestrator instead of triggering recovery

orchestrator/core.rs:200:

core.rs
Lines 200-201
if matches!(e, crate::executive::error::FcpError::ToolFault { .. } | crate::executive::error::FcpError::SchemaViolation(\_)) {
self.recovery_count += 1;
Only ToolFault and SchemaViolation are caught for recovery. If vault:read tries to read a non-existent file, it returns FcpError::Io (line 50 of vault/read.rs), which hits the else branch and aborts the entire orchestrator. This is your [FATAL ERROR] Orchestrator halted: I/O Fault: No such file or directory (os error 2) crash. Every I/O error, ParseFault, and NetworkFault from tools is treated as a system fatality.

4. Most tools are never registered with the Gatekeeper

executive/router.rs:46-62 only registers 3 tools:

vault:read
memory:commit
memory:query
Missing from registration: vault:write, vault:list, memory:stage, system:health, web:fetch, agenda:push, agenda:list, agenda:complete. The LLM is shown schemas for only 3 tools, but even those 3 are fragile because of issue #3. The agent is essentially lobotomized.

5. LLM responses are never pushed back into chat_stack as assistant messages

In orchestrator/core.rs, after the engine returns a response:

Line 162-164: the raw content is sent to the TUI
Line 172: the content is parsed for directives
The response is never added to chat_stack
The LLM has no memory of its own prior responses. Every turn it sees only: system prompt + user message + tool results. Multi-turn conversation is impossible. The model is contextually blind.

P1 - SERIOUS FUNCTIONAL DEFECTS 6. num_ctx is never sent to Ollama

engine/ollama.rs:63-66:

ollama.rs
Lines 63-66
let request = ChatMessageRequest::new(
self.config.model_name.clone(),
chat_messages
).format(FormatType::Json);
The ChatMessageRequest doesn't set num_ctx on the Ollama request. Ollama falls back to its model default (often 2048). Combined with issue #1, the 8192/16384 context window you configured is fiction.

7. message_to_user is never displayed as conversational text

When the LLM returns {"status": "Idle", "message_to_user": "Here's your answer..."}, the orchestrator enters HaltAndAwaitInput(msg) but that arm just resets state and returns. The user sees the raw JSON in the viewport ({"thought": "...", "status": "Idle", ...}) instead of a clean conversational response. There is no TUI event that extracts and renders message_to_user.

8. Heartbeat monitor is dead code

spawn_heartbeat_monitor is defined in orchestrator/heartbeat.rs but never called anywhere. The interrupt_tx/interrupt_rx pair created in the router (line 67-68) has interrupt_tx immediately discarded:

router.rs
Lines 68-68
let _ = interrupt_tx; // Keep alive
The comment says "Keep alive" but let _ = drops it. The entire autonomous idle-task execution system is inert.

9. CancellationToken from main is discarded

router.rs
Lines 6-7
let \_ = cancel_token;
The ctrl+c handler in main.rs fires cancel_token, but the Chat branch throws it away. Ctrl+c exits only because the TUI catches it via crossterm KeyEvent. The snapshot daemon, heartbeat, and any other background processes that should listen for graceful shutdown never get the token.

10. Snapshot daemon is never spawned

spawn_snapshot_daemon exists in memory/ephemeral.rs:137 but is never called. The moka cache is volatile-only. Kill the process and all ephemeral memory is gone.

P2 - LOGGING & OBSERVABILITY (your stated pain) 11. Exactly ONE log statement in the entire codebase

main.rs
Lines 40-40
tracing::info!("Starting FCP Subconscious Orchestrator...");
There is zero tracing in:

Orchestrator loop (state transitions, response parsing, tool dispatch)
Engine (Ollama request/response, token counts, latency)
Gatekeeper (validation failures, tool routing)
Context assembler (prompt construction, identity loading)
Tool execution (what tool, what args, what result)
TUI event handling
You need tracing::debug! or tracing::info! calls at minimum in:

Orchestrator::step() - log state transitions, which tool was called, the raw LLM response
OllamaClient::generate() - log request size, response size, token counts
Gatekeeper::execute_tool() - log tool name, args, validation result
ContextAssembler::assemble() - log whether identity was loaded from file or fallback 12. Errors are swallowed silently in multiple places

let \_ = tx.send(...) throughout orchestrator and router - channel send failures are discarded
tracing::warn!("Semantic Brain offline...") in the router is the only warning, but it doesn't log WHY it failed
The orchestrator's broadcast_state also silently drops send errors
P3 - CORRECTNESS & RULES VIOLATIONS 13. Multiple expect() calls in production code

Your cursorrules says expect() is forbidden outside #[test] blocks. Violations:

src/memory/ephemeral.rs:30 - .expect("Time went backwards")
src/memory/ephemeral.rs:46 - same
src/memory/ephemeral.rs:59 - same
src/memory/ephemeral.rs:121 - same
src/tools/web/fetch.rs:28 - .expect("Failed to build reqwest client")
src/main.rs:29 - unwrap*or_else (this one is actually fine, it's a fallback) 14. vault:write routes core*\* filenames to 00_Core then rejects them

tools/vault/write.rs:75-76: filenames starting with core\_ get routed to "00_Core". Then line 89 calls validate_path_is_mutable which rejects 00_Core. The LLM can never write a file named core_anything.md.

15. scratch.rs and scratch_test.rs are stray files

src/engine/scratch.rs contains a type-exploration snippet (let \_x: String = data.prompt_eval_count;) that doesn't compile on its own. It's not declared in engine/mod.rs so Cargo ignores it, but it's dead weight in the repo.

P4 - LLM ENGINEERING ISSUES 16. The system prompt is too verbose for a 14B model

The context assembler produces a wall of text. A 14B model (qwen2.5:14b) needs a much tighter prompt. The JSON schema should be a concrete example, not a description. The status values explanation is ambiguous (the LLM keeps outputting "Reflect" with empty tool_calls, which triggers recovery). Consider few-shot examples in the prompt.

17. The available_tools_json is double-injected

In OllamaClient::generate(), the available_tools_json is appended to the system message content (line 49). But the caller in orchestrator/core.rs:120 passes "{}" as available_tools_json. Meanwhile, the context assembler ALSO puts the tools schema into the system prompt text (line 40 of context.rs). So either the tools are in the prompt but "{}" is passed (current behavior), meaning line 48-51 of ollama.rs injects {} as available tools, duplicating/confusing things, OR the tools are shown twice.

18. Raw JSON shown to user instead of parsed response

The TUI shows response.content directly (the full JSON blob). Users see {"thought": "...", "status": "Idle", "tool_calls": []} instead of extracted conversational text.

RECOMMENDED FIX ORDER
P0 #1 - Wire AppConfig::load() into the router
P0 #2 - Fix workspace path geometry (remove the extra subdirectory join or fix ignition to create it)
P0 #3 - Expand the orchestrator recovery match to catch Io, ParseFault, NetworkFault from tools
P0 #4 - Register all tools with the gatekeeper
P0 #5 - Push assistant responses back into chat_stack
P2 #11 - Add tracing throughout the orchestrator and engine (you need this before debugging anything else)
P1 #6 - Pass num_ctx to the Ollama request
P1 #7 - Extract message_to_user and display it cleanly in the TUI
P1 #8/#9/#10 - Wire heartbeat, cancellation token, snapshot daemon
Each of these is a single-function or few-line fix. None requires an architectural rewrite. The bones of this system are solid - the cognitive loop, state machine, gatekeeper pattern, tool trait boundary - all well-designed. The problem is the wiring in executive/router.rs which is essentially a half-finished integration harness.

---

Files modified (7 files across 12 issues):

Logging (4 modules instrumented)
src/orchestrator/core.rs - Added tracing to step(): state transitions, bailout checks, LLM response content/tokens, directive parsing, tool dispatch with name+args, recovery triggers, JSON parse failures with raw snippets
src/engine/ollama.rs - Added tracing to generate(): model name, message count, streaming mode, response tokens, errors with context
src/tools/gatekeeper.rs - Added tracing to execute_tool(): authorization checks, registered tool listing on miss, schema validation failures with args, execution timing
src/orchestrator/context.rs - Added tracing to assemble(): identity file loading success/fallback with path, tool count per state
P0 Crash Fixes
src/executive/router.rs (complete rewrite of Chat branch):

P0 #1: AppConfig::load() is now called in main.rs and the Arc<AppConfig> is passed to execute_command. No more AppConfig::default().
P0 #2: Workspace path geometry fixed - ContextAssembler now receives "" as workspace so core_dir = workspace_root/00_Core (matches where ignition puts files)
P0 #4: ALL tools registered: vault:read, vault:write, vault:list, agenda:push, agenda:list, agenda:complete, web:fetch, memory:commit, memory:query
Wired CancellationToken through to orchestrator loop and TUI teardown
Wired spawn_heartbeat_monitor with last_input_time tracking
Wired spawn_snapshot_daemon for ephemeral memory persistence
Interrupted errors from heartbeat no longer kill the loop
src/orchestrator/core.rs:

P0 #3: New is_recoverable_tool_error() method - Io and ParseFault errors from tools now trigger recovery instead of killing the orchestrator
P0 #5: Assistant responses pushed into chat_stack so the LLM retains context across turns
P1 #7: message_to_user extracted and sent to TUI as [ERIS]: ... instead of only showing raw JSON
src/main.rs - Config loaded via AppConfig::load(cli) at startup, passed to router

P1 Fixes
src/engine/ollama.rs - P1 #6: GenerationOptions::default().num*ctx(config.num_ctx) now passed to every Ollama request
P3 Fixes
src/memory/ephemeral.rs - All 4 expect("Time went backwards") replaced with unwrap_or_default()
src/tools/web/fetch.rs - expect("Failed to build reqwest client") replaced with unwrap_or_else(|*| Client::new())
src/executive/cli.rs - Added Clone derive to Cli and Commands
Cargo.toml - url = "2.5" moved from dev-dependencies to main dependencies
Test results: 69/69 pass, clippy clean.
