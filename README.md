# Eris

[![CI](https://github.com/janpauldahlke/eris/actions/workflows/ci.yml/badge.svg)](https://github.com/janpauldahlke/eris/actions/workflows/ci.yml)
[![Rust edition](https://img.shields.io/badge/Rust-Edition%202024-dea584?logo=rust&logoColor=white)](https://doc.rust-lang.org/edition-guide/rust-2024/index.html)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)

**A local-first agent in a single Rust binary: your Markdown vault as memory, grammar-enforced tool calls on llama.cpp, and nothing leaves your machine unless you say so.**

Eris (Episodic Reasoning & Inference System) runs a local LLM as a personal agent over a plain-Markdown vault (Obsidian-compatible). It reads and writes your notes, remembers across sessions through tiered semantic memory, manages reminders and alarms, and calls its tools through a JSON protocol that is **structurally enforced by a GBNF grammar** — no function-calling API required, and no cloud in the loop.

<!-- TODO: 30–60s TUI demo GIF here before public launch -->

## Why

- **Sovereign by architecture, not by promise.** No telemetry leaves the machine. Every outbound integration (web fetch, mail, calendar, Discord) is opt-in and enumerable. The vault is plain text you own; the semantic index is derived data that can always be rebuilt from it.
- **Reliability engineering for small models.** Local 8–26B models are sloppy with JSON. Eris compiles a GBNF grammar per session — and a narrowed per-turn subset grammar — so the model's output is constrained at the token level. Bounded generation (`n_predict_max`) guards against context-window truncation.
- **One brain, three faces.** The same orchestrator and tool registry serve a full-screen terminal UI (ratatui), a localhost web UI (`eris chat --web`), and an optional Discord sidecar sharing the live session.

## What Eris is not

Eris is **not** smarter than hosted frontier models — its intelligence ceiling is the GGUF you run. It is smarter *about your data* than anything hosted: it lives where your notes live, and it works when your network doesn't.

## Core vs. extras

| Core (supported) | Extras (best-effort) |
|---|---|
| Chat (TUI + web), Markdown vault read/write/search | Discord sidecar |
| Tiered memory: staged (ephemeral) + committed (vault) + semantic recall (Qdrant) | Vision (`vision:see`, multimodal GGUF + mmproj) |
| Tool protocol with gatekeeper + GBNF enforcement (llama.cpp) | Voice ingress (STT via ffmpeg) |
| Agenda: reminders, alarms, self-driven follow-ups | Google Workspace mail/calendar tools |
| Web fetch/search with allowlist, consent, and session budget | Moltbook client |

## Quickstart

Prerequisites: Rust (stable, edition 2024), [llama.cpp](https://github.com/ggml-org/llama.cpp) (`llama-server`), a chat GGUF + an embedding GGUF (e.g. `nomic-embed-text`), and Qdrant for semantic memory:

```bash
docker run -d -p 6333:6333 -p 6334:6334 -v eris-qdrant-data:/qdrant/storage qdrant/qdrant
```

Build and run:

```bash
cargo build --release
./target/release/eris chat          # first run launches the ignition wizard
./target/release/eris chat --web    # same session, localhost web UI
```

The first-run wizard writes `.fcp/config.toml` (backend, model paths, GPU layers) and seals the vault directory. Full setup — including the Ollama alternative backend, vision, and voice — is in **[docs/REFERENCE.md](docs/REFERENCE.md)** and **[docs/HOW_TO/](docs/HOW_TO/)**.

### Hardware

| Setup | Works |
|---|---|
| Apple Silicon, 16 GB+ | Good: 7–12B GGUF chat + embed model, Metal offload |
| Apple Silicon, 32 GB+ | Comfortable: 26B-class models, vision mmproj |
| Linux + NVIDIA (8 GB+ VRAM) | Good with `--n-gpu-layers` tuning |
| CPU-only | Runs, but slow; small quantized models only |

## Backends

**llama.cpp is the canonical production backend** — it is the only one with GBNF grammar enforcement, vision, and voice. Ollama is supported as an easier-to-install alternative with weaker JSON discipline (soft `format: json` instead of grammar); expect more recovery turns on long sessions.

## Status

Alpha. Single-user, single-process, developed and dogfooded daily on macOS. Honest known limitations:

- Long-context sessions can still degrade JSON discipline on the Ollama backend (grammar-less path).
- Installation is manual (build from source + fetch models); installers and prebuilt binaries are planned.
- The architecture docs in [docs/updated_architecture/](docs/updated_architecture/) include a frank [self-review](docs/updated_architecture/10_DEEP_REVIEW_2026-07.md) of the codebase's debt — read it before contributing to the orchestrator.

## Documentation

| Doc | Contents |
|---|---|
| [docs/REFERENCE.md](docs/REFERENCE.md) | Complete setup & operations reference (models, config keys, benchmark suite, tool roster) |
| [docs/HOW_TO/](docs/HOW_TO/) | llama.cpp setup, vision, audio, adding a tool, operator manual |
| [docs/updated_architecture/](docs/updated_architecture/README.md) | Code-aligned architecture guide for contributors |

## Contributing

Contributions are welcome under **inbound = outbound** terms: your contributions are licensed under Apache 2.0, confirmed by a [DCO](https://developercertificate.org/) `Signed-off-by` line (`git commit -s`). See [CONTRIBUTING.md](CONTRIBUTING.md) — including the project's non-negotiable engineering rules (zero panics, no `unsafe`, actor-model concurrency).

## License

Copyright 2026 Jan Dahlke. Licensed under the [Apache License, Version 2.0](LICENSE).
