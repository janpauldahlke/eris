# Eris

[![CI](https://github.com/janpauldahlke/eris/actions/workflows/ci.yml/badge.svg)](https://github.com/janpauldahlke/eris/actions/workflows/ci.yml)
[![Rust edition](https://img.shields.io/badge/Rust-Edition%202024-dea584?logo=rust&logoColor=white)](https://doc.rust-lang.org/edition-guide/rust-2024/index.html)
[![codecov](https://codecov.io/gh/janpauldahlke/eris/graph/badge.svg)](https://codecov.io/gh/janpauldahlke/eris)

**Episodic Reasoning & Inference System** — a local, vault-centric assistant: same orchestrator and tools whether you use the **full-screen terminal UI (ratatui)**, **`eris chat --web`** (localhost Axum + SSE), or an **optional Discord sidecar** that shares the live session. Two LLM backends: **Ollama** (default, easiest) or **llama.cpp** (direct GGUF inference with GBNF grammar enforcement). Optional Qdrant holds semantic memory; notes live in a Markdown vault; tools run only through the JSON-schema gatekeeper.

Architecture detail: [docs/updated_architecture/README.md](docs/updated_architecture/README.md).

## Prerequisites

### Rust

- **Stable** toolchain, **Edition 2024** (see `Cargo.toml`).
- Used to compile and run tests from this repo.

### LLM Backend (choose one)

Eris supports two backends. Select during first-run ignition or via `llm_backend` in `.fcp/config.toml`.

#### Option A — Ollama (default)

Eris talks to Ollama over HTTP; defaults match `AppConfig` (`ollama_host`, typically `http://localhost:11434`).

1. **Install** [Ollama](https://ollama.com) for your OS and ensure the daemon is running (`ollama serve`, or the background service the installer sets up).
2. **Pull a chat model** — must match `model_name` in `.fcp/config.toml` (default in code: `gemma4:26b`):

```bash
 ollama pull gemma4:26b
```

Use any tag you prefer; set `model_name` accordingly.

3. **Pull an embedding model** — must match **`embed_model_name`** (default: **`nomic-embed-text`**) for ToolRouter similarity and Qdrant upserts:

```bash
 ollama pull nomic-embed-text
```

4. **Context length:** If you raise `num_ctx` in config, ensure Ollama can serve that context for your model (see Ollama docs for `OLLAMA_CONTEXT_LENGTH` / model limits).

If Ollama is down, chat cannot run.

#### Option B — llama.cpp (GGUF + GBNF grammar)

Direct inference via `llama-server`. Eris manages the server processes, compiles a **GBNF grammar** at session start that makes malformed JSON structurally impossible, and constrains tool arguments per-tool.

1. **Build llama.cpp** from source (requires CMake + C/C++ compiler).
2. **Download GGUF models** for chat and embeddings (e.g. from HuggingFace).
3. **Configure** `llm_backend = "LlamaCpp"` and the `[llama_cpp]` section in `.fcp/config.toml`.

Full instructions: **[docs/LLAMA_CPP_SETUP.md](docs/LLAMA_CPP_SETUP.md)**.

### Qdrant (vector DB)

Used for semantic memory (`memory:query`), boot ingest, and web-artifact cleanup. The client uses the URL in `qdrant_url` (default `http://localhost:6334`). gRPC must be reachable after TCP connect.

**Option A — Docker (typical)**

```bash
docker run -p 6333:6333 -p 6334:6334 qdrant/qdrant
```

- **6333** — REST/dashboard (optional).
- **6334** — gRPC (what Eris uses by default).

**Option B — native/binary** — install Qdrant from upstream and listen on the same ports, or change `qdrant_url` in `.fcp/config.toml`.

If Qdrant is unreachable and `require_semantic_brain` is `true` (default), \*\*chat startup fails\*\* after retries. Set `require_semantic_brain = false` only if you want chat without vector tools.

### Web UI (browser)

`eris chat --web` serves a minimal chat page on **`web_bind_addr` / `web_port`** (see `AppConfig`; example vaults use `127.0.0.1` and `8787`). Figment accepts **`FCP_WEB_BIND_ADDR`**, **`FCP_WEB_PORT`**, and **`FCP_WEB_OPEN_BROWSER`**. Stopping the `eris` process (or the shared cancellation token) tears down the HTTP server and ends the session.

### browser39 (web fetch / search / headlines)

**`web:fetch`**, **`web:find`**, **`web:search`**, and **`news:today`** use the external **[browser39](https://crates.io/crates/browser39)** CLI (subprocess + JSONL; not linked into the `eris` binary).

```bash
cargo install browser39 --locked
browser39 --version   # must succeed on PATH (or set BROWSER39_BIN)
```

Chat startup runs `browser39 --version` when `[web] require_browser39 = true` (default) and seeds `.fcp/browser39/` plus `web_allowlist.toml` under your vault. Operator guide: [docs/WEB_BROWSER39.md](docs/WEB_BROWSER39.md).

### Discord (optional)

With **`[discord]`** in `.fcp/config.toml` (`enabled = true`, **`application_id`**, **`channel_id`** or **`channel_name`**, and a non-empty **`bot_token`**), a Serenity **gateway sidecar** runs in parallel with the active view and forwards a guild text channel into the same orchestrator queue. If Discord is enabled in config but the bot token is missing, chat still runs without the sidecar (see tracing). Details: [docs/updated_architecture/01_BOOTSTRAP_AND_EXECUTIVE.md](docs/updated_architecture/01_BOOTSTRAP_AND_EXECUTIVE.md), [06_UI_TELEMETRY_OPERATIONS.md](docs/updated_architecture/06_UI_TELEMETRY_OPERATIONS.md).

### Google Workspace — Gmail and Calendar (optional)

**`mail:*`** and **`calendar:*`** tools need **`[google]`** with `enabled = true`, `service_account_key`, and `impersonate_user` (Workspace **domain-wide delegation**). In Google Admin → Security → API controls → Domain-wide delegation, authorize the service account client id with at least **`https://mail.google.com/`** and **`https://www.googleapis.com/auth/calendar`**. Enable **Gmail API** and **Google Calendar API** in the same Google Cloud project. See `GoogleConfig` in `src/config.rs`, `src/tools/mail/`, and `src/tools/calendar/`.

### Moltbook (optional)

**`moltbook:*`** tools let Eris participate in Moltbook only when the operator asks. Enable the tool family with `[moltbook] enabled = true`; authenticated tools require `MOLTBOOK_API_KEY` or `api_key_file = "~/.config/moltbook/credentials.json"`. The client pins bearer-token requests to `https://www.moltbook.com/api/v1` and never logs the key.

Registration is explicit: ask Eris to register on Moltbook with a name and description, save the returned API key outside the repo, then finish the claim URL in the browser. Normal visits should start with “check Moltbook”, which routes to `moltbook:home`; Eris does not run an autonomous heartbeat or background poller.

### Checklist

| Piece       | `.fcp/config.toml` keys                         | Notes                                                                     |
| ----------- | ----------------------------------------------- | ------------------------------------------------------------------------- |
| Backend     | `llm_backend`                                   | `"Ollama"` (default) or `"LlamaCpp"`                                     |
| Ollama HTTP | `ollama_host`                                   | Default `http://localhost:11434` (Ollama backend)                         |
| Chat model  | `model_name`                                    | Match what you `ollama pull` (default `gemma4:26b`)                       |
| Embed model | `embed_model_name`                              | Default `nomic-embed-text` (768-d vectors → Qdrant)                       |
| llama.cpp   | `[llama_cpp]` table                             | `home`, model paths, ports, GPU layers — see [LLAMA_CPP_SETUP.md](docs/LLAMA_CPP_SETUP.md) |
| Qdrant URL  | `qdrant_url`                                    | Default `http://localhost:6334` (gRPC)                                    |
| Web UI      | `web_bind_addr`, `web_port`, `web_open_browser` | Loopback + port for `eris chat --web`; optional `FCP_WEB_*` env overrides |
| Discord     | `[discord]` table                               | Optional; needs `bot_token` + app id + channel when `enabled = true`      |
| Google WS   | `[google]` (`enabled`, `service_account_key`, `impersonate_user`) | Optional `mail:*` + `calendar:*`; Cloud APIs + Admin domain-wide delegation |
| Moltbook    | `[moltbook]` (`enabled`, `api_key_file`)        | Optional `moltbook:*`; prefer `MOLTBOOK_API_KEY` or an operator-owned credentials file |
| Web fetch / headlines | `news_today_enabled`, `[web]` + `.fcp/web_allowlist.toml` (MVP) | **`web:fetch`** / **`web:find`** / **`web:search`** (when `search_enabled`); **`news:today`**. Requires **browser39** on PATH — verified at chat startup; see [browser39 (web fetch)](#browser39-web-fetch--search--headlines) and [docs/WEB_BROWSER39.md](docs/WEB_BROWSER39.md). |

Figment also merges `FCP_` environment variables over TOML (e.g. `FCP_WORKSPACE`, `FCP_LOG_LEVEL`, `FCP_USER_NAME`). For other fields, match `AppConfig` in `[src/config.rs](src/config.rs)` to the env key shape your Figment build expects.

**Installing a release binary (PATH, first-run wizard, day-to-day use):** [docs/HOW_TO/END_USER_README.md](docs/HOW_TO/END_USER_README.md).

## Workspace initialization

1. **Choose or create a directory** that will be the vault (notes, `.fcp/`, etc.).
2. `cd` into that directory — configuration and paths are resolved from the \*\*current working directory\*\*, not from `FCP_VAULT` alone for normal chat.
3. **First run:** if `.fcp/seal` is missing and stdin is a TTY, an optional **setup welder** may run first (environment probes, vault-root confirmation); then the **ignition** wizard scaffolds identity and config. It creates `.fcp/`, **`00_Invariants/`** (and the rest of the v2 vault layout), and writes `config.toml`.
4. **Config:** edit `.fcp/config.toml` as needed (model name, `num_ctx`, Qdrant URL, `workspace` id for collection **`fcp_vault_v2_{workspace}`**, web bind/port, Discord block, etc.). Environment overrides use the `**FCP_`\*\* prefix (e.g. `FCP_WORKSPACE`).

Multi-machine note: copy or recreate `.fcp/config.toml` per environment; keep the same `workspace` string if you want the same Qdrant collection name.

## Usage

```bash
cd /path/to/your/vault
/path/to/eris chat
```

Same vault, browser UI:

```bash
cd /path/to/your/vault
/path/to/eris chat --web
```

Common flags (see `eris chat --help`):

- **`-w` / `--workspace`** — logical partition (Qdrant collection suffix, ephemeral snapshot id). Env: `FCP_WORKSPACE` (default `default`).
- **`-v` / `--vault`** — legacy/config override for `vault_root` in `AppConfig`; normal chat still expects you to **launch from** the vault directory.
- **`--web`** — localhost web chat (Axum + SSE) instead of ratatui.

Verbose tracing: **`-V`**, **`-VV`**.

## Benchmarking

Measure **protocol quality**, **tool use**, and **latency** on your real stack: benchmarks drive the same **orchestrator + gatekeeper + Ollama** path as interactive chat (user prompts → full turns with tools), not a standalone mock loop. Use saved JSON under **`<vault>/.fcp/benchmarks/`** to compare models or track regressions over time.

---

### What runs

| Piece | Role |
| ----- | ---- |
| **Scenario harness** | For each scenario step: push a user line, run `Orchestrator::step`, then score assistant JSON (`tool_calls`, `message_to_user`) against expectations. |
| **Speed probe** | One Ollama chat round-trip for throughput / timing labels (complements scenario wall time). |
| **Artifacts** | JSON (and optional Markdown) reports keyed by timestamp + model name; `--list` shows runs for the **current vault cwd**. |

**Duration** depends on model size, GPU, and suite size — count **full LLM turns**, not seconds. Treat published “~N minute” estimates as rough when moving between hardware.

---

### Suites

Run from your vault directory (`cd` into the vault that owns `.fcp/config.toml`).

| Suite | Scenarios | Intent |
| ----- | --------- | ------ |
| **`quick`** | 5 | Sanity: JSON protocol, memory stage, vault read, system health, clock. |
| **`standard`** | 9 | **`quick`** plus multi-hop chains, memory query, adversarial noise. |
| **`comprehensive`** | 15 | **`standard`** plus unicode / nested JSON / large listings / recovery / branching. |

```bash
eris benchmark                          # same as --suite standard
eris benchmark --suite quick
eris benchmark --suite comprehensive
```

Common flags: **`--format`** `table` \| `json` \| `markdown`, **`--output`** `<path>` (write report file), **`--isolation`** `strict` \| `relaxed` \| `unsafe`.

**VRAM / RAM after a run:** when **`unload_ollama_models_on_chat_exit`** is `true` in `.fcp/config.toml` (default), Eris runs **`ollama stop`** for the chat and embedding models after a benchmark finishes — same behavior as exiting chat when Ollama was already running on the host. If Eris itself spawned an `ollama serve` child for the run, that process is torn down instead and unload is skipped.

**Per-scenario time budget:** set **`benchmark_scenario_timeout_secs`** in `.fcp/config.toml` (default **120**). The harness uses `max(that value, each scenario’s built-in timeout)`, so slow models (large weights, layer offloading, CPU offload) get enough wall time without editing scenario sources. Align it with **`generation_timeout_secs`** if you raise LLM ceilings.

---

### Safety and isolation

- **Gatekeeper + isolation mode** limit which tools can run (mutating mail/Moltbook/calendar-style tools are blocked in **`strict`**).
- **Benchmarks use your configured vault** (`active_vault` / cwd): scenarios read real paths like `00_Invariants/Identity.md`. Do not assume a throwaway copy unless you point `eris` at an isolated vault directory.
- Optional **`--compare`** after a run loads the **previous** saved report for the same vault and prints a diff table.

Modes:

| Mode | Meaning |
| ---- | ------- |
| **`strict`** (default) | Safe tools only (e.g. memory:\*, vault read/search/list, system:\*, clock:\*). |
| **`relaxed`** | Adds read-only external families where configured (e.g. weather, wiki). |
| **`unsafe`** | Widest tool surface; only use with **`--no-dry-run`** and **`--i-understand-risks`** when you accept real side effects. |

---

### Listing, comparing, trends

```bash
# Index of runs for this vault (shows run IDs for --diff)
eris benchmark --list

# Same vault: two run IDs from --list (baseline .. current)
eris benchmark --diff '2026-05-09_12-45-30_model-a..2026-05-09_14-30-22_model-b'

# Two sibling vault folders (cwd = parent of both): compare each vault's *latest* saved report
cd vaults
eris benchmark --diff-vaults gemma nemo

# Alias
eris benchmark --diff-siblings gemma nemo

# Explicit JSON paths (any machine / naming)
eris benchmark --diff-files ./vault-a/.fcp/benchmarks/baseline.json ./vault-b/.fcp/benchmarks/current.json

# Trend table from the last N saved reports (optional Markdown file)
eris benchmark --trend 10
eris benchmark --trend 10 --output quality-trend.md
```

After a normal run, **`--compare`** diffs against the latest stored report (see “Safety” above).

---

### Metrics (report summary)

| Area | Examples |
| ---- | -------- |
| **Quality** | JSON parse success rate, tool-call validity, timeouts, scenario pass/fail. |
| **Speed** | Probe-based prompt/gen throughput and phase timings (see report footnotes for definitions). |
| **Scenarios** | Per-scenario duration, rounds, and success bit — useful for “model A nails multi-hop, model B does not”. |

---

### Example: two models, two vaults

Run benchmarks inside each vault (each vault has its own `.fcp/config.toml` / model):

```bash
cd vaults/gemma && eris benchmark --suite standard
cd ../nemo      && eris benchmark --suite standard
```

Compare **latest** reports without hand-picking paths — from the **parent** of the vault directories:

```bash
cd vaults
eris benchmark --diff-vaults gemma nemo
```

Or compare explicit JSON files:

```bash
eris benchmark --diff-files \
  vaults/gemma/.fcp/benchmarks/<run-id>.json \
  vaults/nemo/.fcp/benchmarks/<run-id>.json
```

The CLI prints a side-by-side comparison (quality + speed columns); use JSON exports for dashboards or CI.

## Program flow

**Mental model — data and interaction flow** (one chat turn, simplified):

```mermaid
flowchart LR
    subgraph you["Operator"]
        KB[Input]
    end
    subgraph surfaces["Presentation"]
        TUI[TUI ratatui]
        WEB[Web UI SSE]
        DISC[Discord channel]
    end
    subgraph process["eris process"]
        ORC[Orchestrator]
        GK[Gatekeeper + tools]
        EPH[Ephemeral cache]
    end
    subgraph daemons["Local daemons"]
        OLL[Ollama]
        LCPP[llama-server chat + embed]
        QD[Qdrant]
    end
    subgraph vaultdir["Vault cwd"]
        FS[Markdown tree]
        META[.fcp config / agenda / alarms]
        LOG[telemetry logs]
    end

    KB --> surfaces
    surfaces <-->|UserAction / SessionEvent| ORC
    ORC -->|chat JSON + router embeds| OLL
    ORC -->|chat JSON + GBNF grammar| LCPP
    ORC --> GK
    GK --> FS
    GK --> EPH
    GK -->|memory:query upsert| QD
    ORC -->|boot ingest| QD
    ORC -.->|tracing| LOG
    META -.->|read at startup| ORC
```

You interact through the **TUI**, a **localhost web page**, and/or **Discord**; all paths funnel **`UserAction`** into the same orchestrator task and receive **`SessionEvent`** updates (see `src/presentation/`). The orchestrator calls **Ollama** or **llama-server** (depending on `llm_backend`) for structured JSON and uses **ToolRouter** embeddings for pre-LLM gating. The llama.cpp path compiles a **GBNF grammar** at session start that constrains output to valid protocol JSON with per-tool argument schemas. **Tools** run only through the **gatekeeper**: they read/write **Markdown**, use **ephemeral** staging, and hit **Qdrant** for semantic memory. **Logs** go to `.fcp/telemetry/` — not mixed into the chat deck.

- **Terminal:** Full-screen **ratatui** UI under `src/ui/terminal/`: chat deck, status, telemetry; `Ctrl+C` exits and tears down daemons this process started.
- **Web:** `src/ui/web/` — Axum router, SSE stream of `SessionEvent`, small static JS; suitable for the same machine or SSH port-forward.
- **Discord:** Optional Serenity sidecar in `src/ui/discord/`; assistant lines are `try_send` to a bounded queue from the presentation multiplexer when enabled.
- **Logs:** Rotating files under **`<vault>/.fcp/telemetry/logs/`** (tracing); not printed to the TUI buffer for normal operation.
- **Semantics:** If Qdrant is reachable, boot may **ingest** markdown into **`fcp_vault_v2_{workspace}`**. If not and `require_semantic_brain` is true, startup fails; if false, chat runs without vector tools.
- **Developers:** New tools and gatekeeper rules: [docs/ADDING_A_TOOL.md](docs/ADDING_A_TOOL.md).

## Natural language → tool routing (phrase compendium)

Tool choice is **not** parsed from rigid commands. The orchestrator’s **ToolRouter** (`[src/orchestrator/tool_router.rs](src/orchestrator/tool_router.rs)`) embeds your text with the same model as vector memory (`embed_model_name` in config, default `nomic-embed-text`) and compares it to **precomputed** vectors—one per tool built from the tool name, JSON-schema description, and (when present) **`routing_hints`** from the embedded TOML descriptors in `[src/tools/specs.rs](src/tools/specs.rs)`. If a tool has no descriptor hints, **`routing_phrases::fallback_triggers`** in `[src/tools/routing_phrases.rs](src/tools/routing_phrases.rs)` supplies compile-time “typical phrasing” for embeddings and the slim phrase compendium. Tools whose **cosine similarity** meets `tool_match_threshold` in `.fcp/config.toml` (default **0.50**) are surfaced to the LLM. In slim tool mode the **`[FCP_TOOL_PHRASE_MAP]`** snippet is generated at runtime from registered tools plus those descriptors ([`src/orchestrator/context/compendium.rs`](src/orchestrator/context/compendium.rs)); the table below is the human-readable mirror (keep it aligned with `routing_phrases.rs` / `specs.rs`). **Web tools** need **browser39** on PATH — see [browser39 (web fetch)](#browser39-web-fetch--search--headlines).

The **gatekeeper** only enforces **state** and **JSON Schema** on tool calls (`[src/tools/gatekeeper.rs](src/tools/gatekeeper.rs)`); it does not map phrases to tools.

When **`db:find_connections`** or any **`calendar:*`** tool is in the current tool roster, **`[SESSION_REFERENCE_TIME]`** is appended to the system prompt (same wall clock as `clock:now`, built in [`src/tools/clock/now.rs`](src/tools/clock/now.rs) via [`src/orchestrator/context/assembler.rs`](src/orchestrator/context/assembler.rs)) so RFC3339 fields do not need a guessed year.

**Extra rules (outside pure similarity):**

- **Short utterances** (≤3 words or ≤15 characters) are treated as chat-only unless you include a URL, a leading `/`, a domain-like token (e.g. `news.ycombinator.com`), or explicit web wording such as `search the web` / `look up online`.

Representative **`routing_hints`** (say things _like_ this—the model still decides, and similarity is fuzzy):

| Tool                       | Typical phrasing                                                                                                 |
| -------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| **vault:list**             | list files, show directory, browse folder, what files exist                                                      |
| **vault:read**             | read file, open note, show file, inspect markdown                                                                |
| **vault:write**            | save note, write file, append note, create markdown                                                              |
| **vault:taglist**          | list vault tags, map of tags, tag frequencies, synthesis taxonomy, notes under tag                                |
| **memory:query**           | search memory, do you remember, what is my name, who am I, user preferences, my identity, recall context         |
| **memory:stage**           | remember this, stage memory, temporary memory, hold in staging                                                   |
| **memory:staged_list**     | show staged memory, list staged ids, what is staged                                                              |
| **memory:commit**          | commit staged memory, persist one memory, save to vault, keep forever                                            |
| **memory:commit_all**      | commit all memories, flush staged memory, bulk commit staged                                                     |
| **agenda:push**            | add task, remind me, todo, queue task                                                                            |
| **agenda:list**            | show tasks, list agenda, pending tasks                                                                           |
| **agenda:remove**          | remove task, cancel agenda, delete from list, drop task, never mind                                              |
| **agenda:remind_at**       | remind me at/in/about, remember to, nudge/ping me at, snooze, on my agenda or todo list, task reminder           |
| **agenda:remind_self**     | set a self reminder, resume this workflow in 10 minutes, self-driven loop, wake me with checklist/plan            |
| **agenda:complete**        | task done, complete task, mark done, finished the …                                                              |
| **web:fetch**              | open website, read web page, fetch a URL, look up this link — plus pasted URLs and lexical web wording |
| **web:search**             | search the web, google this, look up online, find on the internet, latest news search (allowlisted fetch of results; needs `[web] search_enabled`) |
| **news:today**             | today’s headlines, top stories, morning briefing, news digest, breaking news, front page, politics/science/business/world/UK sections; homepage listing + optional top-article fetch (not for arbitrary one-off URLs—use **web:fetch**) |
| **web:find**               | search fetched page chunks in vault mission cache (after **web:fetch**); use **best_match_url** for the next fetch |
| **system:health**          | health check, system status, CPU/memory usage, Ollama status, diagnostics                                        |
| **clock:now**              | what time is it, current time, timezone, date and time                                                           |
| **clock:timer**            | in 30 minutes, countdown, generic timer, label-only reminder (not agenda)                                        |
| **clock:alarm**            | wake me up, alarm clock only, standalone alarm, no todo                                                          |
| **weather:current**        | weather now, temperature outside, is it raining, current conditions                                              |
| **weather:forecast**       | forecast, hourly, next days, will it rain tomorrow                                                               |
| **wiki:summary**           | Wikipedia, encyclopedia, what is X, who was, define (topic—not a URL)                                            |
| **db:find_connections**    | train from/to, Zugverbindung, ICE/IC/RE, Deutsche Bahn, next connection, platforms, delays, city-to-city transit |
| **mail:check**             | check email, inbox, unread, new mail, who emailed me                                                             |
| **mail:read**              | read email, open message, full email, message content                                                            |
| **mail:write**             | send email, compose mail, reply, email to                                                                        |
| **mail:digest**            | summarize email, today’s mail, digest, recap inbox                                                               |
| **mail:delete**            | delete email, trash message, discard                                                                             |
| **mail:move**              | move to folder, label email, file under, move to spam                                                            |
| **skills:list**            | list skills, what skills are available, show skills, skill index                                                 |
| **skills:read**            | read skill, show skill details, inspect skill by id                                                              |
| **skills:create**          | create skill, add skill, author skill, update skill with overwrite                                               |
| **calendar:list**          | Google Calendar, meetings today, this week’s schedule, appointments, what’s on my calendar, list events, am I free |
| **calendar:get**           | open this calendar event, event details by id, full meeting JSON, read Google Calendar event                      |
| **calendar:create**        | add calendar event, schedule meeting, block time, create Google Calendar appointment                              |
| **calendar:update**        | reschedule meeting, change event time, rename meeting, edit calendar event                                       |
| **calendar:delete**        | cancel meeting, delete calendar event, remove from Google Calendar                                                 |
| **moltbook:home**          | check Moltbook, visit Moltbook, catch up on Moltbook, Moltbook heartbeat                                         |
| **moltbook:feed**          | browse Moltbook feed, read submolt, following feed, Moltbook posts                                               |
| **moltbook:search**        | semantic search Moltbook, find posts by meaning, discover discussions by topic                                   |
| **moltbook:comment/post/vote** | comment on Moltbook, post to Moltbook, upvote Moltbook; only after explicit operator intent or approval      |
| **moltbook:dm**            | Moltbook DM, direct messages, inbox, DM request, reply to Moltbook message                                       |

To change operator-facing routing text, prefer **`routing_hints`** in `[src/tools/specs.rs](src/tools/specs.rs)`; for tools without TOML hints, edit **`fallback_triggers`** in `[src/tools/routing_phrases.rs](src/tools/routing_phrases.rs)`. The lexical phrase lists inside `tool_router.rs` remain for URL/page detection and short-input guards (not the full tool roster).

## New agenda + vault flows

- **`agenda:remind_self`** creates a self-driven reminder cycle (plan + optional checklist) that wakes the agent with `SELF REMINDER` framing when the alarm fires; use it for autonomous multi-step follow-up, not user Done/Snooze reminders.
- **`vault:taglist`** provides a taxonomy map for `30_Synthesis/` frontmatter tags (`tag -> count`, optional paths) so you can browse “unknown unknowns” before running keyword search.
- `vault:taglist` cache is persisted at `.fcp/tools/taglist.json`, built once at startup, and lazily rebuilt after successful `vault:write` operations under `30_Synthesis/*.md` (or via `refresh: true`).


---

introspect full tcp of llamacpp ` sudo tcpdump -i lo -s 0 -l -A 'tcp port 8090'`

---

## Copyright

Copyright (c) 2026 Jan Dahlke. All Rights Reserved.

## Contributors and IP

To retain the unilateral ability to monetize or dictate the future open-source license of the project, you must implement a **Contributor License Agreement (CLA)** or a **Copyright Assignment** agreement before any trusted contributor makes their first commit.

- **Copyright assignment:** The contributor legally transfers total ownership of their code to you.
- **Broad CLA:** The contributor retains their copyright but grants you a perpetual, irrevocable, worldwide, royalty-free license to use, modify, distribute, and re-license their contributions both commercially and non-commercially.

### Automated CLA on GitHub

This repository uses [**Contributor Assistant**](https://github.com/contributor-assistant/github-action) via [`.github/workflows/cla.yml`](.github/workflows/cla.yml). On each pull request, contributors who are not on the workflow **allowlist** must sign using the phrase the bot posts; signatures are stored in [`signatures/version1/cla.json`](signatures/version1/cla.json). The legal text they agree to is in **[`docs/CLA.md`](docs/CLA.md)**.

**Configure after merge:** Edit the workflow if your default branch is not `main`, if the repo moves (update the `path-to-document` raw URL), or if your GitHub username is not `janpauldahlke` (allowlist). If **branch protection** prevents the bot from committing signature updates, create a **personal access token** with `repo` scope, add it as repository secret `PERSONAL_ACCESS_TOKEN`, and uncomment the corresponding line in the workflow.

**Security note:** That workflow uses `pull_request_target` by design; do not extend it with steps that build or execute unchecked PR code.
