# Updated architecture (code-aligned)

This folder is a **developer and agent** guide to the `eris` crate (`src/`). It reflects the codebase as of the repository state when authored; prefer `src` and `Cargo.toml` for ground truth.

## Reading order

| Doc | Contents |
|-----|----------|
| [00_OVERVIEW.md](./00_OVERVIEW.md) | Mental model, layer diagram, glossary |
| [01_BOOTSTRAP_AND_EXECUTIVE.md](./01_BOOTSTRAP_AND_EXECUTIVE.md) | `main`, CLI, config, vault paths, ignition, peripherals |
| [02_ORCHESTRATOR_LAYER.md](./02_ORCHESTRATOR_LAYER.md) | `Orchestrator::step`, state machine, routing, context view |
| [03_ENGINE_LLM_AND_ROUTING.md](./03_ENGINE_LLM_AND_ROUTING.md) | `LlmEngine`, Ollama, ToolRouter, optional reasoning FSM |
| [04_MEMORY_SUBSYSTEM.md](./04_MEMORY_SUBSYSTEM.md) | Ephemeral cache, Qdrant semantic brain, ingest, snapshot daemon |
| [05_TOOLS_GATEKEEPER_DESCRIPTORS.md](./05_TOOLS_GATEKEEPER_DESCRIPTORS.md) | `Tool` trait, registry, gatekeeper, embedded descriptors |
| [06_UI_TELEMETRY_OPERATIONS.md](./06_UI_TELEMETRY_OPERATIONS.md) | TUI, events, alarms, heartbeat, logging, preflight |
| [07_CROSS_CUTTING.md](./07_CROSS_CUTTING.md) | Errors, async patterns, workspace rules reminder |
| [08_SELF_REVIEW.md](./08_SELF_REVIEW.md) | Author notes, caveats, possible doc drift |
| [09_CRITICAL_REVIEW.md](./09_CRITICAL_REVIEW.md) | Critical engineering review: debt, refactors, redesign triggers |

## Binary and CLI naming

- **Cargo package / binary:** `eris` (default binary name matches `[package].name`).
- **Clap program name:** `eris` (`#[command(name = "eris")]` in `executive/cli.rs`).

Documentation may still say “FCP” for the product concept; the invoked binary and `--help` name are `eris`.
