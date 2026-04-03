# Eris

**Episodic Reasoning & Inference System** — a local, vault-centric TUI assistant: Ollama for chat/embeddings, optional Qdrant for semantic memory, Markdown vault on disk, tools behind a JSON-schema gatekeeper.

## Scope

- **In scope:** Interactive `chat` from a vault directory, tool use (vault, memory, web, agenda, clocks, etc.), structured logging under `.fcp/telemetry/logs/`.
- **Out of scope today:** `eris run` and `eris tool` are present in the CLI but **not implemented** for production use; use `chat`.

Architecture detail: [docs/updated_architecture/README.md](docs/updated_architecture/README.md).

## Prerequisites

| Requirement | Role |
|-------------|------|
| **Rust** (2024 edition; stable toolchain) | Build and test |
| **Ollama** | LLM + embedding API (`ollama serve`) |
| **Qdrant** | Vector store for `memory:query` and ingest (gRPC; default often `http://localhost:6334`) |

Optional: `docker` only if your Qdrant setup uses containers (see your own ops).

## Installation

From the repository root:

```bash
cargo build --release
```

The binary is `target/release/eris` (package name `eris`).

```bash
cargo test
```

## Workspace initialization

1. **Choose or create a directory** that will be the vault (notes, `.fcp/`, etc.).
2. **`cd` into that directory** — configuration and paths are resolved from the **current working directory**, not from `FCP_VAULT` alone for normal chat.
3. **First run:** if `.fcp/seal` is missing, the app runs an **ignition** wizard (model, identity scaffold). It creates `.fcp/`, `00_Core/`, standard folders, and config.
4. **Config:** edit `.fcp/config.toml` as needed (model name, `num_ctx`, Qdrant URL, `workspace` id for collection `fcp_vault_{workspace}`, etc.). Environment overrides use the **`FCP_`** prefix (e.g. `FCP_WORKSPACE`).

Multi-machine note: copy or recreate `.fcp/config.toml` per environment; keep the same `workspace` string if you want the same Qdrant collection name.

## Usage

```bash
cd /path/to/your/vault
/path/to/eris chat
```

Common flags (see `eris chat --help`):

- **`-w` / `--workspace`** — logical partition (Qdrant collection suffix, ephemeral snapshot id). Env: `FCP_WORKSPACE` (default `default`).
- **`-v` / `--vault`** — legacy/config override for `vault_root` in `AppConfig`; normal chat still expects you to **launch from** the vault directory.

Verbose tracing: **`-V`**, **`-VV`**.

## Expected outcome

- **Terminal:** Full-screen **ratatui** UI: chat deck, status, telemetry; `Ctrl+C` exits and tears down daemons this process started.
- **Logs:** Rotating files under **`<vault>/.fcp/telemetry/logs/`** (tracing); not printed to the TUI buffer for normal operation.
- **Semantics:** If Qdrant is reachable, boot may **ingest** markdown into the collection `fcp_vault_{workspace}`. If not and `require_semantic_brain` is true, startup fails; if false, chat runs without vector tools.
- **Developers:** New tools and gatekeeper rules: [docs/ADDING_A_TOOL.md](docs/ADDING_A_TOOL.md).
