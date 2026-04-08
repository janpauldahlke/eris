# Critical review (engineering)

Audience: senior Rust engineers and anyone designing LLM control planes. Opinionated. Not a roadmap commitment.

---

## 1. Verdict in one paragraph

The codebase is **coherent for a single-binary, local-first agent**: clear error taxonomy, channel-based UI, explicit gatekeeper, tests around the orchestrator. The main liabilities are **scale** (`orchestrator/core.rs` ~2k lines), **product surface** (`Run`/`Tool` stubs, `FcpError` vs `eris` naming), **duplicate routing knowledge** (embeddings + giant `match` hint strings + TOML descriptors), and **latent config debt** (`enable_reasoning_fsm` unused, vector size implicit). None of that requires a rewrite; it requires **surgical extraction and honesty in the CLI**.

---

## 2. Structural problems

### 2.1 `Orchestrator::step` and friends

`core.rs` is the cognitive center and a **review bottleneck**. The inner loop mixes: pre-LLM routing, system prompt assembly, `build_llm_view`, generation, condensation, directive parsing, tool batch execution, recovery, and TUI side effects. **Low risk to extract** (no behavior change): `run_pre_llm_routing`, condensation trigger, and “first LLM call vs tool loop” into separate modules or inherent methods on a smaller `StepContext` struct. **High risk** to “re-architect” into a generic state machine without tests—your existing tests are valuable; keep them green.

### 2.2 `executive/router.rs` chat command

Hundreds of lines in one `match` arm: wiring channels, watchers, gatekeeper registration, orchestrator spawn, TUI. This is **startup composition**, not business logic—but it is hard to navigate. **Low-hanging fruit:** `fn bootstrap_chat(...) -> Result<ChatRuntime>` in a `router` submodule or `executive/chat_bootstrap.rs` that returns handles. Same symbols, easier testing of registration order without running the full TUI.

### 2.3 Semantic brain size and coupling

`memory/semantic.rs` is large and touches Qdrant, Ollama embeddings, ingest, search, filters. Acceptable for now. **Redesign only if** you add a second vector backend or need swap tests—then introduce a `VectorStore` trait at the boundary (your project rules already allow that pattern for mocks).

---

## 3. LLM / agent architecture

### 3.1 JSON-in-chat as the only contract

Forcing **one JSON object per turn** with Ollama `FormatType::Json` is a reasonable constraint. Weaknesses:

- **Fragile** if the model emits preamble or multiple JSON objects; you already slice JSON with `find`/`rfind`—that is pragmatic but not robust against adversarial or sloppy models.
- **Recovery** depends on `Recover` + system injects; that is correct, but the number of edge cases grows with `core.rs`.

**Low-hanging fruit:** centralize JSON extraction in one function with unit tests (valid JSON, junk prefix, fenced code—decide policy explicitly).

### 3.2 ToolRouter: embeddings + lexical duplication

Pre-LLM routing uses **user text** embedding against **precomputed tool vectors**. `enrich_for_routing` also embeds a **large manual `match` per tool name** for lexical hints. Embedded **TOML descriptors** add `routing_hints` again. Three sources of truth drift.

**Honest fixes (pick one direction, not all three):**

- **Minimal:** delete or shrink the `match` arms; rely on `routing_hints` + short description from descriptors only (single compile-time source).
- **Slightly more work:** generate the embedding text from descriptor TOML at build time (macro or `build.rs`) so Ollama never sees stale paraphrases.

**Do not** add a second embedding model without measuring latency and cost.

### 3.3 “No semantic match” still enables full tool roster

When the router returns **no** similarity hits, you still run **tool mode with all schemas**—by design. That avoids silent “I can’t use tools” but **costs context** and increases model confusion. Alternatives are product decisions: cap roster, or require explicit `/tool` or keyword to escalate. Worth a design discussion, not a quick refactor.

### 3.4 Dead `ReasoningRouter` and `enable_reasoning_fsm`

`engine/router.rs` is **test-only**; config carries `enable_reasoning_fsm` but production does not use it. **Low-hanging fruit:** delete the flag from `AppConfig` or wire streaming; otherwise you are lying to operators.

### 3.5 `LlmEngine::generate(..., available_tools_json)` — always empty

`OllamaClient` injects tools into the **first system message**; the second parameter is unused in the hot path. **Low-hanging fruit:** document that contract on the trait, or remove the parameter if nothing will use it—avoids confusion for the next implementer.

---

## 4. Memory and correctness

### 4.1 Qdrant vector dimension

Collection creation hardcodes **768** dimensions. If `embed_model_name` changes to a model with different width, you get **runtime failure** or silent mismatch depending on Qdrant behavior. **Low-hanging fruit:** assert dimension from a first embedding probe at startup, or store in config.

### 4.2 Ephemeral `moka::iter()` for title lookup

`get_by_title` scans the cache. Fine at 10k capacity; if you stage heavily, **O(n)** shows up. Only optimize if profiling says so.

### 4.3 Snapshot daemon vs semantic cleanup

The daemon ties expired web artifacts to Qdrant deletes. Good. Failure modes are **best-effort** (warn logs)—acceptable for local; document that semantic orphans are possible if Qdrant is down during expiry.

**Promotion/decay vs `step()`:** tier evaluation runs on a separate tick from snapshotting; it is **skipped** while `Orchestrator::step` holds `promotion_suppressed_during_step` so slow generations do not interleave with cache mutation. This is a deliberate `Arc<AtomicBool>` exception to “single owner” wording—still no mutex on the orchestrator struct.

---

## 5. Naming and operator experience

- **`FcpError`** and **`FCP_*` env** while the binary is **`eris`**: confusing for new contributors. Renaming the error type is a **large churn**; **low-hanging fruit** is consistent user-facing strings (`main.rs` still logs “FCP Subconscious Orchestrator”).
- **`Commands::Run`** and **`Commands::Tool`** are **stubs**. Either implement, or remove from clap until ready—**honest CLI** beats surprising no-ops.

---

## 6. Testing and CI

- **Strength:** orchestrator unit tests cover directives, condensation, interrupts, agenda heuristics.
- **Gap:** no automated **e2e** against real Ollama in CI (understandable). Wiremock covers HTTP tools well.
- **Gap:** `Tool`/`tool` CLI path does not exercise the real gatekeeper.

---

## 7. Low-hanging fruit (prioritized)

1. **Rename or fix startup log strings** (`main.rs`, log file prefix `fcp_core.log` if you care about branding consistency).
2. **Consolidate ToolRouter hint strings** with descriptors (remove duplicate `match`).
3. **Extract** `bootstrap_chat` from `router.rs` for readability only.
4. **Vector dimension** check or config validation at startup.
5. **Remove or wire** `enable_reasoning_fsm` / `ReasoningRouter`.
6. **Clarify** `LlmEngine` trait parameters in docs or API.
7. **Stub subcommands:** hide, implement, or return explicit `FcpError::Config("not implemented")` with a tracking issue.

---

## 8. When a redesign is justified

| Trigger | Direction |
|--------|-----------|
| Second LLM backend (not Ollama) | Stabilize `LlmEngine` + streaming story; split adapter crates if you must ship sizes |
| Multi-user / server mode | Today’s design is single-process; you would need session isolation and auth |
| Tool count >> 30 | Tiered router becomes mandatory; consider tool groups in config, not more embeddings |
| Formal agent protocol (JSON-RPC, MCP) | Replace ad-hoc TUI channels with a protocol layer—**large**; do not half-do it |

---

## 9. What not to do

- **Do not** introduce `Arc<Mutex<Orchestrator>>` to “simplify” tests—violates your own concurrency rules.
- **Do not** abstract every tool behind dynamic traits for the sake of “cleanliness”—you already have the right boundary (`Tool`).
- **Do not** merge microservices or add Redis for this use case—local-first is the product.

---

## 10. Relation to [08_SELF_REVIEW.md](./08_SELF_REVIEW.md)

`08` is **documentation accuracy**. This file is **engineering judgment**. Update both if you change routing, CLI, or vector assumptions.
