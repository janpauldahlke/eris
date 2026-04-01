# Project F.C.P.

## 1. The Manifesto: Zero-Bloat Autonomy

Current autonomous agents fail because they treat text as the universal interface for control. They stuff system prompts with rules, dump full roadmaps into the context window, and hope the LLM doesn't hallucinate. This guarantees context exhaustion, attention dilution, and catastrophic failure on Small Language Models (SLMs).

**Project F.C.P.** is a single-binary, local-first Rust orchestrator engineered to run x-B parameter models on consumer hardware (16GB VRAM) without cognitive collapse. We discard prompt-engineering in favor of mathematical state enforcement, asynchronous trait abstraction, and strict context gating.

---

## 2. The Two-Layer Architecture

The system operates across an impenetrable membrane. The LLM is trapped in Layer 1, completely blind to the orchestration mechanics of Layer 2.

### Layer 1: The Conscious Engine (LLM Trait)

- **Trait Abstraction:** Abstracted behind an asynchronous `LlmEngine` Rust trait to decouple the orchestrator from the inference backend.
- **Inference:** Powered in V1 by a local Ollama daemon via HTTP (`ollama-rs`), ensuring cross-platform compilation without C++ linker nightmares or FFI pointer traps.
- **Context Diet:** Sees only a minimized prompt, the running stack, and a heavily curated JSON ToolSet dynamically injected per turn.

### Layer 2: The Subconscious (Rust Orchestrator)

- **Deterministic FSM:** The state machine manages the token budget, intercepts and parses internal `<think>` blocks, and executes the physical Rust logic.
- **The Gatekeeper:** Physically strips irrelevant tools from the LLM's schema payload based on the active `AgentState`. The LLM cannot hallucinate tools it cannot mathematically see.

---

## 3. The Core Innovations

### A. The Semantic Firewall (Dynamic Context Diet)

Instead of forcing the LLM to process 50 capabilities, F.C.P. uses state-aware routing. Tool schemas injected into the Ollama API payload expand and contract dynamically. If the `moka` cache is empty, the `memory:commit` tool is physically withheld. This enforces absolute cognitive alignment and zero-shot accuracy with minimal token overhead.

### B. The 3-Tier Memory Engine (Native Rust Topology)

We manage the context budget through strict physical layers:

1. **The Stack (Tier 1 - Context Window):** Managed by the orchestrator; condensed/wiped when nearing the `num_ctx` threshold.
2. **The Ephemeral (Tier 2 - moka Cache):** TTL-based staging ground holding extracted facts in RAM. If not explicitly committed, they evaporate.
3. **The Vault (Tier 3 - Qdrant + Obsidian):** Authoritative Markdown files indexed via `nomic-embed-text`. Uses `memory:query` for surgical semantic retrieval instead of dumping massive files into context.

### C. The Native Gatekeeper (No MCP Bloat)

We eliminated MCP JSON-RPC servers to avoid network latency. Tools are native asynchronous Rust functions. The LLM interfaces via strict `schemars`-derived JSON payloads piped through the orchestrator. Any validation failure triggers the **S04.2 Fuckup Loop**, forcing the model to self-correct based on the Rust error trace.

---

## 4. `_1_MASTERGRID`: The F.C.P. Implementation Map


| Block ID  | Subsystem                          | Core Physics & Constraints                                                                                                    |
| --------- | ---------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| **S00.2** | **Global Configuration**           | `AppConfig` via `clap`. Resolves `vault_root.join(workspace)`. Defines `num_ctx` and hardware timeouts.                       |
| **S01**   | **The Engine & Router**            | `ollama-rs` wrapper. Native reasoning extraction (`<think>`) routing to prevent UI bloat.                                     |
| **S03**   | **The Vault & Vector Abstraction** | `notify` file watcher. Whitelisted Drop Zone (`90_Drops/`). Clones `_template` and drops `.fcp_seal.json`.                    |
| **S03.1** | **Ephemeral Memory**               | Tier 2 `moka` RAM cache. Time-stamped staging for facts/sensory alerts before disk commitment.                                |
| **S04**   | **The Subconscious Orchestrator**  | `AgentState` FSM. Manages the `[CHAT_STACK]` and routes LLM control flow.                                                     |
| **S04.2** | **The "Fuckup" Recovery Loop**     | Traps `ParseFault` and `SchemaViolation`. Hard bailout at `max_recovery_attempts`.                                            |
| **S04.3** | **Autonomy & Heartbeat**           | Shifts to `Idle` on timeout. Async Guillotine drops HTTP future on keystroke via `tokio::select!`.                            |
| **S05**   | **The Tool Gatekeeper**            | State-adaptive gating. Traps path-traversal (`../` or `00_Core/`). Exposes CLI dispatch (`fcp tool`).                         |
| **S05.1** | **Gatekeeper Logic & Guards**      | Derives JSON schemas via `schemars`. Native API injection. Semantic Guards prevent infinite loops.                            |
| **S06**   | **Core Tool Manifest**             | `memory:query` (Semantic Zoom), `system:health`, `memory:stage`, `memory:commit`, `vault:read` (Map Fallback), `vault:write`. |
| **S06.1** | **External Capabilities**          | `web:fetch` via `reqwest`. Markdown scraping + Truncation Guillotine.                                                         |
| **S06.2** | **Temporal Memory**                | Hidden `.fcp_agenda.json` FIFO queue. `agenda:push`, `agenda:list`, `agenda:complete`.                                        |
| **S07**   | **The Cockpit (TUI)**              | `ratatui` + `crossterm`. Non-blocking `mpsc` event loop. ASCII Avatars map 1:1 to `AgentState`.                               |
| **S08**   | **Deployment & Pre-Flight**        | Daily rotated `tracing` to `.fcp/logs/`. Blocking boot checks for Ollama/Qdrant. LTO release profile.                         |
| **ROOT**  | `**.cursorrules`**                 | **Laws of the Hull:** Zero Panics (`unwrap` banned). Absolute `unsafe` ban. `tempfile` for FS tests.                          |


