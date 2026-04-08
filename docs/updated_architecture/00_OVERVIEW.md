# Overview and mental model

## What this program is

**Eris** is a local, vault-centric assistant: a Rust binary that connects a **terminal UI (TUI)** to an **Ollama** chat model, wraps tools behind a **gatekeeper** (JSON Schema validation + per-state allowlists), optionally routes “which tools matter” via **embedding similarity** (ToolRouter), and persists long-term recall in **Qdrant** (semantic memory) while keeping short-lived staging in an **moka** cache (ephemeral memory).

The **active vault** is always the process **current working directory** at config load—not `vault_root + workspace` from TOML. That is a deliberate mental model: `cd` into your vault, run chat, `.fcp/` and markdown live beside your notes.

## Architectural layers (simplified)

```mermaid
flowchart TB
    subgraph ui["UI layer"]
        TUI["ratatui TuiApp"]
        EVT["mpsc: TuiEvent / UserAction"]
    end

    subgraph orch["Orchestrator layer"]
        ORC["Orchestrator::step"]
        CA["ContextAssembler"]
        CV["build_llm_view"]
        TR["ToolRouter optional"]
    end

    subgraph eng["Engine layer"]
        LLM["OllamaClient : LlmEngine"]
        OLL["Ollama HTTP API"]
    end

    subgraph tools["Tools layer"]
        GK["Gatekeeper"]
        TREG["ToolRegistry HashMap"]
    end

    subgraph mem["Memory layer"]
        EPH["EphemeralMemory moka"]
        SEM["SemanticBrain Qdrant"]
    end

    TUI --> EVT
    EVT --> ORC
    ORC --> CA
    ORC --> CV
    ORC --> TR
    TR --> OLL
    ORC --> LLM
    LLM --> OLL
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
    participant T as TUI
    participant O as Orchestrator
    participant E as OllamaClient

    M->>R: Chat + config + cancel token
    R->>T: channels + terminal
    R->>O: spawn loop + step on input
    T->>O: UserAction::Submit
    O->>O: pre-LLM routing, assemble context
    O->>E: generate JSON response
    E-->>O: assistant JSON
    O->>O: parse, tools, gatekeeper, stack
    O->>T: TuiEvent deck / state
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

## Source map (`src/`)

| Directory | Role |
|-----------|------|
| `executive/` | CLI, command routing, ignition, peripherals, identity helpers |
| `config.rs` | `AppConfig` + Figment load |
| `vault_layout.rs` | Paths under `.fcp/` |
| `workspace.rs` | `init_workspace` for multi-workspace vault roots (legacy/bootstrap) |
| `engine/` | `LlmEngine`, Ollama, token metrics, reasoning FSM |
| `orchestrator/` | `core/` loop, `state`, `context/` (assembler, LLM view, condensation, compendium), `llm_support/` (JSON envelope + post-tool copy), `tool_router`, `heartbeat/`, `alarms/`, `loop/` policies |
| `memory/` | Ephemeral + semantic |
| `tools/` | Trait, gatekeeper, tool implementations, descriptors |
| `ingest/` | Chunking helpers for semantic pipeline |
| `telemetry/` | tracing init, preflight, routing log codes |
| `ui/` | TUI app, render, events |
| `util/` | HTTP API client, fs watch |

## Out of scope for this doc set

- **`target/`** build artifacts
- **Specific vault contents** (e.g. `vaults/eve/`): layout and conventions are described generically
