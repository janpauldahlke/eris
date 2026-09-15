# Overview and mental model

## What this program is

**Eris** is a local, vault-centric assistant: a Rust binary that connects **presentation surfaces**—full-screen **ratatui** (`eris chat`), **localhost web + SSE** (`eris chat --web`), and optionally **Discord**—to one shared orchestrator backed by **Ollama**, **llama.cpp** (direct GGUF inference with GBNF grammar enforcement), or **OpenRouter** (hosted OpenAI-compatible API; strict JSON-schema `response_format` instead of GBNF, env-only API key, embeddings stay local). Tools sit behind a **gatekeeper** (JSON Schema + per-state allowlists); pre-LLM “which tools matter” uses **embedding similarity** (ToolRouter) via a backend-agnostic `EmbeddingProvider` trait. Long-term recall lives in **Qdrant**; short-lived staging uses an **moka** cache (ephemeral memory).

The **active vault** is always the process **current working directory** at config load—not `vault_root + workspace` from TOML. That is a deliberate mental model: `cd` into your vault, run chat, `.fcp/` and markdown live beside your notes.

## Architectural layers (simplified)

```mermaid
flowchart TB
    subgraph ui["Presentation layer"]
        TUI["ui/terminal TuiApp"]
        WEB["ui/web Axum+SSE"]
        DISC["ui/discord sidecar"]
    end

    subgraph orch["Orchestrator layer"]
        ORC["Orchestrator::step"]
        CA["ContextAssembler"]
        CV["build_llm_view"]
        TR["ToolRouter optional"]
    end

    subgraph eng["Engine layer"]
        LLM["OllamaClient : LlmEngine"]
        LCPP["LlamaCppClient : LlmEngine"]
        ORTR["OpenRouterClient : LlmEngine"]
        OLL["Ollama HTTP API"]
        LSRV["llama-server chat + embed"]
        ORAPI["openrouter.ai hosted API"]
        GRAM["GBNF grammar compiler"]
        JSCH["structured/ JSON-schema compiler"]
    end

    subgraph tools["Tools layer"]
        GK["Gatekeeper"]
        TREG["ToolRegistry HashMap"]
    end

    subgraph mem["Memory layer"]
        EPH["EphemeralMemory moka"]
        SEM["SemanticBrain Qdrant"]
    end

    TUI -->|UserAction| ORC
    WEB -->|UserAction| ORC
    DISC -->|UserAction| ORC
    ORC -->|SessionEvent| TUI
    ORC -->|SessionEvent| WEB
    ORC -.->|mux / assistant lines| DISC
    ORC --> CA
    ORC --> CV
    ORC --> TR
    TR --> OLL
    ORC --> LLM
    ORC --> LCPP
    ORC --> ORTR
    LLM --> OLL
    LCPP --> LSRV
    ORTR --> ORAPI
    GRAM --> LCPP
    JSCH --> ORTR
    ORC --> GK
    GK --> TREG
    ORC --> EPH
    TREG --> EPH
    TREG --> SEM
```

## Main runtime flow (chat)

```mermaid
sequenceDiagram
    participant M as main
    participant R as router execute_command
    participant V as Terminal or Web (+ optional Discord mux)
    participant O as Orchestrator
    participant E as LlmEngine (Ollama, LlamaCpp, or OpenRouter)

    M->>R: Chat + config + cancel token
    R->>V: presentation_tx + view setup
    R->>O: spawn loop + step on input
    V->>O: UserAction (Submit / SubmitIngress / …)
    O->>O: pre-LLM routing, assemble context
    O->>E: generate JSON response (+ GBNF grammar if llama.cpp, JSON-schema response_format if OpenRouter)
    E-->>O: assistant JSON
    O->>O: parse, tools, gatekeeper, stack
    O->>V: SessionEvent deck / state / errors
```

## Glossary

| Term | Meaning |
|------|---------|
| **Vault root / active vault** | `AppConfig::config_source_dir` (= cwd at load); `active_vault()` |
| **Workspace** | Logical id for Qdrant collection `fcp_vault_v2_{workspace}`, ephemeral snapshot filename `.fcp/ephemeral_{workspace}.bin`, etc. |
| **Layer 1 / Layer 2** | Legacy docs sometimes call the LLM “Layer 1” and orchestrator+tools “Layer 2”; code modules are `engine` and `orchestrator` |
| **chat_stack** | Canonical `Vec<Message>`; LLM may see a *view* via `build_llm_view` |
| **Tool mode vs conversational** | Pre-LLM routing: some user turns skip tools (short input, system alarm prefix); else tools enabled with full or slim schemas |
| **Gatekeeper** | Validates args against JSON Schema and enforces `AgentState` allowlists |
| **LlmBackend** | `Ollama` (default), `LlamaCpp`, or `OpenRouter`; set in `AppConfig` via `llm_backend` |
| **EmbedBackend** | `Ollama` or `LlamaCpp` (`embed_backend` in config) — embeddings are always local, decoupled from the chat backend; `resolved_embed_backend()` defaults to "same as chat when local, else Ollama" |
| **GBNF grammar** | BNF-style grammar passed to llama-server to constrain output to valid FCP protocol JSON; compiled at session start from registered tool schemas |
| **response_format (OpenRouter)** | Per-turn strict JSON Schema (`json_schema` → `json_object` → off downgrade ladder on HTTP 400) compiled from the same offered-tool schemas as the GBNF subset; `provider.require_parameters = true` prevents fail-open routing |
| **EmbeddingProvider** | Trait (`engine/embedding.rs`) abstracting vector generation; `OllamaEmbedding` and `LlamaCppEmbedding` implement it (keyed off `embed_backend`, not `llm_backend`) |
| **40_MEDIA** | Vault subtree of `media.json` catalog cards for user-uploaded blobs; Qdrant indexes card text only when `[vision] enabled` |

## Source map (`src/`)

| Directory | Role |
|-----------|------|
| `executive/` | CLI, command routing, ignition, peripherals, identity helpers |
| `config.rs` | `AppConfig` + Figment load |
| `vault_layout.rs` | Paths under `.fcp/` |
| `workspace.rs` | `init_workspace` for multi-workspace vault roots (legacy/bootstrap) |
| `engine/` | `LlmEngine` trait, `OllamaClient`, `LlamaCppClient`, `OpenRouterClient`, shared OpenAI wire helpers (`openai_wire.rs`), `EmbeddingProvider` trait, token metrics (+ reasoning tokens / cost), reasoning FSM |
| `engine/grammar/` | GBNF grammar compiler: static envelope (`envelope.rs`), tool name enum (`tool_names.rs`), JSON Schema → GBNF per-tool args (`schema_to_gbnf.rs`) |
| `engine/structured/` | OpenRouter structured output: `schemars` → OpenAI JSON-schema subset (`schema_to_openai.rs`), FCP envelope schema builder (`envelope_schema.rs`) |
| `orchestrator/` | `core/` loop, `state`, `context/` (assembler, LLM view, condensation, compendium), `llm_support/` (JSON envelope + post-tool copy), `tool_router`, `heartbeat/`, `alarms/`, `loop/` policies |
| `memory/` | Ephemeral + semantic |
| `media/` | `40_MEDIA` catalog cards (`media.json`), embed text for Qdrant |
| `tools/` | Trait, gatekeeper, tool implementations, descriptors |
| `ingest/` | Chunking helpers for semantic pipeline |
| `telemetry/` | tracing init, preflight, routing log codes |
| `presentation/` | View-neutral `UserAction`, `SessionEvent`, `InputSource`, alarm → action relay, presentation multiplexer |
| `ui/terminal/` | ratatui `TuiApp`, render, crossterm setup |
| `ui/web/` | Axum router, SSE, browser chat |
| `ui/discord/` | Optional Serenity gateway sidecar |
| `util/` | HTTP API client, fs watch |

## Out of scope for this doc set

- **`target/`** build artifacts
- **Specific vault contents** (e.g. `vaults/eve/`): layout and conventions are described generically
