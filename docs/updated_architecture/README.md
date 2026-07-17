# Updated architecture (code-aligned)

This folder is a **developer and agent** guide to the `eris` crate (`src/`). It is maintained against `src/` and `Cargo.toml`; when they disagree, **code wins**—update these files after refactors that touch bootstrap, orchestrator, memory, or TUI contracts.

## Reading order

| Doc | Contents |
|-----|----------|
| [00_OVERVIEW.md](./00_OVERVIEW.md) | Mental model, layer diagram, glossary |
| [01_BOOTSTRAP_AND_EXECUTIVE.md](./01_BOOTSTRAP_AND_EXECUTIVE.md) | `main`, CLI, config, vault paths, ignition, peripherals |
| [02_ORCHESTRATOR_LAYER.md](./02_ORCHESTRATOR_LAYER.md) | `Orchestrator::step`, state machine, routing, `context/`, `llm_support/`, alarms, heartbeat |
| [03_ENGINE_LLM_AND_ROUTING.md](./03_ENGINE_LLM_AND_ROUTING.md) | `LlmEngine` (Ollama + LlamaCpp), `EmbeddingProvider`, GBNF grammar compiler, ToolRouter |
| [04_MEMORY_SUBSYSTEM.md](./04_MEMORY_SUBSYSTEM.md) | Ephemeral cache, Qdrant semantic brain, ingest, snapshot daemon |
| [05_TOOLS_GATEKEEPER_DESCRIPTORS.md](./05_TOOLS_GATEKEEPER_DESCRIPTORS.md) | `Tool` trait, registry, gatekeeper, embedded descriptors |
| [06_UI_TELEMETRY_OPERATIONS.md](./06_UI_TELEMETRY_OPERATIONS.md) | Presentation types (`SessionEvent` / `UserAction`), TUI (`ui/terminal`), web UI (`ui/web`), optional Discord sidecar, multiplexer, alarms, conditional idle heartbeat, logging, preflight |
| [07_CROSS_CUTTING.md](./07_CROSS_CUTTING.md) | Errors, async patterns, workspace rules reminder |
| [08_SELF_REVIEW.md](./08_SELF_REVIEW.md) | Author notes, caveats, possible doc drift |
| [09_CRITICAL_REVIEW.md](./09_CRITICAL_REVIEW.md) | Critical engineering review: debt, refactors, redesign triggers |
| [10_DEEP_REVIEW_2026-07.md](./10_DEEP_REVIEW_2026-07.md) | Whole-codebase deep review (Jul 2026): quantified god-component audit, long-context JSON root cause, prioritized cleanup plan |
| [11_OSS_SHIPPING_ROADMAP.md](./11_OSS_SHIPPING_ROADMAP.md) | Apache 2.0 open-sourcing & shipping roadmap: license mechanics, engine decision, packaging strategy, M0–M3 milestones |
| [12_META_STRATEGY.md](./12_META_STRATEGY.md) | Meta strategy: claim-by-claim product evaluation, positioning, audiences, content flywheel, launch sequence, funding |
| [13_MEMVID_AND_MEMORY_SIMPLIFICATION.md](./13_MEMVID_AND_MEMORY_SIMPLIFICATION.md) | Memvid as vector-store candidate + deleting the promotion-tier ladder; phased A–E plan |

## Binary and CLI naming

- **Cargo package / binary:** `eris` (default binary name matches `[package].name`).
- **Clap program name:** `eris` (`#[command(name = "eris")]` in `executive/cli.rs`).

Documentation may still say “FCP” for the product concept; the invoked binary and `--help` name are `eris`.
