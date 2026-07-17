# Deep review — July 2026 (whole codebase)

Scope: all of `src/` (~64k LOC across ~240 files). Vaults deliberately not read.
Method: four parallel structural audits (orchestrator, engine/JSON, memory/tools, executive/UI/config), with the highest-impact claims re-verified by hand against the source. Findings marked **[verified]** were confirmed line-by-line, not just reported.

This file supersedes nothing; it complements [09_CRITICAL_REVIEW.md](./09_CRITICAL_REVIEW.md), which remains directionally correct. This review is more quantified and adds the long-context JSON root-cause analysis.

---

## 1. Verdict

The codebase is disciplined where your rules demand it and undisciplined where they don't:

- **The Absolute Laws hold.** Zero `unwrap`/`expect`/`panic!` in production paths across memory, tools, engine, orchestrator (all hits are inside `#[cfg(test)]`). No `unsafe`. No TODO/FIXME/HACK litter. Error taxonomy is consistently `FcpError` + `?`. **[verified]**
- **The Orchestrator is a confirmed god component:** 47 fields, ~12 subsystems, a 25-argument constructor, and a ~640-line `step()`. The file splits under `core/` are mostly *fake extraction* — 10 of 11 files are still `impl Orchestrator` methods on `&mut self`, so nothing actually got decoupled.
- **Your llama.cpp long-context JSON problem is not a grammar problem.** It is a truncation problem: unbounded generation (`n_predict: -1`), no reserved completion window, and a chars/4 token estimate that under-counts. The GBNF grammar guarantees *shape*, not *completion* — the model starts a valid envelope and gets cut off. The recovery loop then injects more text and makes the next attempt worse. **[verified]** (§3)
- **Real drift bugs exist** where the same knowledge is maintained in two+ places: the slim tool-offer list (prompt vs grammar) **[verified]**, tool descriptions (impl vs `specs.rs`, ~48/63 diverge), gatekeeper allowlists (4 copy-pasted match arms), config keys (Rust + 3 web-console files), Qdrant vector dims (hardcoded 768 twice).
- **Operator honesty gaps:** `config.log_level` and `-V` are silently ignored **[verified]**; `eris run` is a silent no-op **[verified]**; `eris tool memory:query` fakes success.

None of this needs a rewrite. It needs ~15 surgical changes, ordered below (§8).

---

## 2. The Orchestrator god component (quantified)

`src/orchestrator/core/orchestrator.rs:56–133` — **47 fields** on one struct, `new()` takes **25 positional args**.

Fields by concern: tools/routing (11), config/budgets (6), memory (5), UI/presentation (5), context assembly (4), telemetry timings (4), Moltbook domain state (3), web domain state (2), chat transcript (2), engine (2), control plane (2), state machine (1). Domain-specific state (`moltbook_browse_ledger`, `tool_repeat_failure_streak`, `web_tool_calls_this_turn`, `pending_weather_deck_report`) living on the core turn object is the clearest sign policy leaked upward.

### `step()` is still the god method

`core/step.rs:39–679`, ~640 LOC, one inner loop mixing: entry reset → prefetch → pre-LLM routing → recovery/tool-cap bailouts → **four-way context assembly branch (lines 205–337)** → proactive condensation → LLM view + GBNF subset selection → generate + interrupt select → parse recovery → deck emit → post-gen condensation → directive → tool batch dispatch.

### The extractions are file splits, not decoupling

10 `impl<E: LlmEngine> Orchestrator<E>` blocks across `step.rs`, `tool_dispatch.rs`, `turn_entry.rs`, `transitions.rs`, `deck.rs`, `llm_directive.rs`, `condensation.rs`, `helpers.rs`, `pre_llm_routing.rs`, `orchestrator.rs`. Everything still has full mutable access to all 47 fields, so responsibilities cannot be reasoned about per-file. The only genuine peels are `loop/` (pure policy enums — good seed) and `moltbook_browse_ledger.rs` (self-contained type).

### Confirmed drift bug (P0) **[verified]**

The slim offered-tool list is built **twice, differently**:

- Prompt assembly: inline in `step.rs:242–271` — Moltbook union + `doc:read`→`vault:write` pairing, **no `web:find` pairing**.
- GBNF grammar: `slim_offered_tool_names` in `llama_gbnf_subset.rs:74–119` — same logic **plus** `web:find` when `web:fetch`/`web:search` is offered (lines 101–110).

The comment at `llama_gbnf_subset.rs:73` ("Same offered list as slim assembly") is false. Result: the grammar can permit a tool the model was never offered in the prompt. Fix: delete the inline copy in `step.rs` and call `slim_offered_tool_names` from both paths.

### Other duplication inside the orchestrator

- Truncation helpers ×3: `helpers::trim_chars`, `view::trim_snippet`, `view::cap_log_string`.
- Condensation thresholds in three places: proactive estimate (`step.rs:339–363`), reactive real-token (`step.rs:560–575`), ceilings inside `condensation.rs:96+`.
- Dual directive entry points: production uses `directive_from_parsed` (`step.rs:582`) while `process_llm_response` (`llm_directive.rs:38`) survives as a parallel path used only by tests.
- Token-budget logic is fragmented across `window.rs` (estimators), `step.rs` (two triggers), `helpers.rs` (`WEB_CONDENSATION_THRESHOLD`, trim budgets), and `view.rs` (char snippets) — no single owner.

---

## 3. llama.cpp JSON discipline on long contexts — root cause

Your symptom ("JSON discipline is not that good on really long contexts") decomposes into a causal chain, each link verified or evidenced:

1. **Unbounded generation.** Every llama.cpp request sends `n_predict: Some(-1)` (`engine/llama_cpp.rs:338`) with no `max_tokens`, no stop sequences, no reserved completion budget. **[verified]**
2. **No pre-send context accounting.** Nothing checks `prompt_tokens + reserved_completion < num_ctx` before dispatch. The proactive condensation trigger uses `estimate_stack_tokens` = `chars/4 + 4` (`context/window.rs:37–44`), a proxy that under-estimates for code/JSON-heavy stacks. Real `usage.prompt_tokens` comes back from the server but is never used to calibrate.
3. **Grammar guarantees shape, not completion.** GBNF constrains next-token choice; if the server hits the context end mid-derivation, output is a *prefix* of a valid envelope — invalid JSON. `json-string ::= "\"" json-char* "\""` (`grammar/envelope.rs:22`) is unbounded, so a long `thought` can starve the closing braces. No `finish_reason` handling exists on the response (`llama_cpp.rs` `Choice` has only `message`/`delta`).
4. **Recovery amplifies the failure.** Parse failure → `RecoverFromFuckup` → a large recovery system message is pushed onto the stack (`transitions.rs:103–106`) and the next hop may force full tool schemas (`turn_entry.rs:247`) — i.e. the response to "prompt too long" is *a longer prompt*, up to `max_recovery_attempts` (default 3).
5. **One genuinely grammar-free LLM path:** condensation summarizer calls run with `attach_session_grammar: false` (`core/condensation.rs:162–171`) and invalid summarizer JSON is silently wrapped as summary text (`window.rs:350–362`).

`json_envelope.rs` is ~1000 lines because it compensates for all of the above with ~8 distinct extraction/repair mechanisms (balanced-brace walker, `find`/`rfind` fallback, best-effort key extractors, prose tool-name scan, think-strip, idle salvage…). None of them can repair a truncated object. It also imports `tools::vault::write::VaultWriteArgs` — protocol↔tools layering leak.

### Fixes, in order of leverage

1. **Reserve a completion window:** before each generate, require headroom (512–1024 tokens) against `num_ctx`; condense first if absent. Calibrate the chars/4 proxy with the previous hop's real `usage.prompt_tokens`.
2. **Cap `n_predict`** to remaining context (or a fixed 1024–2048) instead of `-1`.
3. **Classify truncation distinctly:** serde "EOF while parsing" / unbalanced braces ⇒ *context truncated* ⇒ condense + retry at same temperature — do **not** enter the protocol-fault Recover path that injects guidance text.
4. **Bound envelope strings in GBNF** (`thought`, `message_to_user`) or cap via `n_predict`; today the grammar allows infinite strings.
5. **Give condensation calls a tiny grammar** (or a hard `n_predict` cap + strict parse) instead of no grammar.
6. Stop arming full tool schemas after a protocol fault unless tool names were actually extracted from the bad output.

---

## 4. Config: 2650-line god-file with dead controls

`src/config.rs`: 16 config types, **113 fields on `AppConfig`**, ~139 `default_*` fns, ~530 lines of tests, imported by ~72 files.

Dead or lying controls **[verified]**:

- `config.log_level` and CLI `-V/--verbose` are **never applied** — `telemetry/logger.rs:17–18` uses `RUST_LOG` or hardcoded `eris=debug,fcp=debug`. Your log files are always debug-noisy regardless of TOML.
- `Commands::Run { prompt }` discards the prompt and returns `Ok(())` (`executive/router.rs:563–565`). `Commands::Tool` fakes success for `memory:query` (`router.rs:567–573`). Preflight still probes daemons for `Run`, so it can fail preflight and then do nothing.
- `AppConfig.vault_root` / `-v` is written during load but `active_vault()` only uses `config_source_dir` (`config.rs:2098–2104`) — the flag doesn't select the chat vault.
- `discord.public_key`: zero non-config references.

Note: `enable_reasoning_fsm`, called dead in `09_CRITICAL_REVIEW.md` §3.4, **is now used** (`engine/llama_cpp.rs:32–38, 302–303` gates `enable_thinking`). 09 is stale on this point. `engine/router.rs` (`ReasoningRouter`) remains production-dead.

Split mechanically into `config/` (features / llm / memory_context / document_rag+web_ui / load / defaults) with `AppConfig` as a re-exporting facade — no call-site churn.

---

## 5. Tool subsystem: 63 tools, 5–8 edit points each

Adding one tool touches, minimum: the impl file, family `mod.rs`, `chat_session.rs` registration (~lines 425–805), `gatekeeper.rs` state-allowlist arms + `known_tools` test, and a `specs.rs` TOML blob; optionally `routing_phrases.rs`, `registration.rs`, `normalize_tool_args`. That edit surface is why things drift:

- `Tool::description()` vs `specs.rs` `short_description`: **~48 of 63 tools have diverged wording.**
- `routing_phrases.rs` fallbacks overlap `routing_hints` in `specs.rs` for most tools; 18 tools rely on specs only.
- Gatekeeper `known_tools` test (`gatekeeper.rs:926–987`) lists 59 of 63 — missing `vision:see`, `vision:display`, `media:catalog`, `media:meta`.
- `registration.rs` doesn't register anything (feature gates only) — rename candidate.

Gatekeeper reality check: Chat state is effectively open (denies only `agenda:complete`); Reflect/Idle/Recover are real policy. Two intentional elevation paths exist (`force_full_tool_schemas` dispatch and Recover→Chat elevation at `gatekeeper.rs:198–211`) — fine by design *for user turns*, but they mean Chat allowlisting is the only line of defense in the common case, and the Recover elevation leaks Chat privileges into autonomous turns (see §10.3).

Highest-ROI fixes: one CI test asserting impl names ⊆ specs ⊆ gatekeeper ⊆ known_tools; data-driven allowlist table instead of 4 `matches!` arms; dedupe `WeatherCityArgs` (`weather/current.rs` and `forecast.rs` are near-identical); move `specs.rs` blobs to `descriptors/*.toml` + `include_str!`.

Non-findings worth recording: the 14-file `web/` cluster (~4.5k LOC) maps to real concerns (allowlist, consent, ledger, cache, browser39 fetcher) — do not collapse it. It shares nothing with `moltbook/` (authenticated JSON API client) — do not merge them. The actual formatting bloat is `weather/report.rs` at 771 lines for Open-Meteo output.

---

## 6. Memory

- `semantic.rs` (1608 lines, ~19% tests) is a coherent but oversized facade: Qdrant lifecycle (165–275), embeddings/CRUD (277–401), vault ingest (403–690), search (696–1038), pure helpers (1041–1295). Clean seams; split into `qdrant_client` / `vault_ingest` / `memory_query` behind the `SemanticBrain` facade when convenient.
- **Vector-dim footgun:** collection creation hardcodes **768** in both `semantic.rs:184–186` and `document_store.rs:~92`, while `validate_embedding_provider_vs_qdrant` can disagree later. Create should use `EmbeddingProvider::dimensions()`. (09 §4.1 called this "partially resolved" — the create path is still hardcoded.)
- `document_store.rs` duplicates the Qdrant lifecycle patterns of `semantic.rs` — extract a shared `qdrant_util` when splitting.
- Error handling quality is good throughout: soft-fail ingest when embeddings are down, consistent taxonomy, zero production panics.

---

## 7. UI / presentation / misc dead weight

- Alarm-payload→UserAction mapping exists **three times**: `presentation/multiplex.rs`, `ui/web/bridge.rs`, and an inline duplicate in `ui/terminal/app.rs:177–198`. Make the TUI call `alarm_relay`; consider always using the multiplexer with Discord optional and retiring `bridge.rs`.
- `ui/web/tools_config_schema.rs` (975) + `tools_config_merge.rs` + `settings_merge.rs` hand-mirror `AppConfig` keys. Adding one config knob = 4 files. Long-term: derive the console schema from a single field registry.
- `src/bin/moltbook_soak_check.rs` is orphaned — not in `Cargo.toml` `[[bin]]`. Register or delete.
- `benchmark/` (~4.5k LOC) is **not** dead: wired to `eris benchmark` and CI. Keep.
- `src/generated/gws_types` is checked-in generated code (manifest 2026-04-15), used by mail/calendar. Fine.
- Naming drift: binary `eris`, but `FcpError`, `FCP_*` env, `fcp_core.log`, "Starting FCP Subconscious Orchestrator…" in `main.rs:58`. Cosmetic; do last.
- Small dead code: `ARTIFACT_QUERY_SNIPPET_CHARS` (`context_view_hint.rs:7–9`) references a tool that is now `web:find`; two prefetch helpers are `pub` but only used locally.

---

## 8. Consolidated cleanup plan (do in this order)

### P0 — correctness bugs and lying controls (hours each)

| # | Change | Where |
|---|--------|-------|
| 1 | Unify slim tool-offer: delete inline copy, call `slim_offered_tool_names` from assembly too | `step.rs:242–271`, `llama_gbnf_subset.rs:74` |
| 2 | Cap `n_predict`; reserve completion window before generate; use real `usage.prompt_tokens` to calibrate estimates | `llama_cpp.rs:338`, `step.rs:339–363`, `window.rs:37` |
| 3 | Treat EOF/unbalanced parse failures as *truncation* → condense+retry, not protocol Recover | `step.rs:488–549`, `json_envelope.rs` |
| 4 | Wire `config.log_level` (+`-V`) into `init_tracing`, or delete both | `telemetry/logger.rs:17` |
| 5 | Remove or implement `Run`/`Tool` subcommands; align preflight | `router.rs:563–573`, `cli.rs:105`, `preflight.rs` |
| 6 | Use `EmbeddingProvider::dimensions()` at collection create (both stores) | `semantic.rs:184`, `document_store.rs:~92` |

### P1 — drift-prevention (a day each)

7. CI inventory test: impl names ⊆ `specs.rs` ⊆ gatekeeper ⊆ `known_tools` (also fixes the 4 missing vision/media entries).
8. Data-driven gatekeeper allowlists (one table, four views) — and key authorization on **turn origin**, not on live `self.state` (see §10.4; fixes the Recover privilege leak for autonomous turns).
9. Shared `truncate_with_ellipsis`; dedupe `WeatherCityArgs`; TUI uses `alarm_relay`; delete `moltbook_soak_check.rs` or register it; kill `ReasoningRouter` and the unused `available_tools_json` trait param.
10. Bound `thought`/`message_to_user` in the GBNF envelope; attach a minimal grammar to condensation calls.

### P2 — real decoupling of the orchestrator (a week+, tests must stay green)

11. Peel pure functions off `&mut self` (`helpers` → free fns over `&mut Vec<Message>`; collapse `process_llm_response` into the prod path).
12. `ToolBatchExecutor`: move `execute_tool_batch` off `Orchestrator` into a struct borrowing only gatekeeper/stack/ledgers/config; return decision + effects.
13. `ContextBudget` service owning `num_ctx`, both condensation triggers, and estimator calibration — single owner for token math.
14. Extract `assemble_system_prompt_for_hop` from `step.rs:205–337`; then reduce `step()` to a coordinator over named stages. Group the 47 fields into nested structs (`TurnLimits`, `Telemetry`, `DomainLedgers`, `MemoryHandles`) as an intermediate step.
15. Move domain state off the core: Moltbook ledger/streaks → `moltbook/session_policy`, weather deck stitch → weather module, alarms/heartbeat out of `orchestrator/`.

### P3 — cosmetic / large-churn (only when bored)

16. Split `config.rs` into `config/` facade; derive web-console schema from one registry.
17. Split `semantic.rs`; shared Qdrant bootstrap; split `moltbook/actions.rs` and `gatekeeper.rs` by concern.
18. Naming pass: `ErisError` alias, `ERIS_*` env with `FCP_*` fallback, log-file rename.

### What not to do (unchanged from 09, still correct)

- No `Arc<Mutex<Orchestrator>>`, no trait-abstracting every tool, no microservices/Redis.
- Do not merge `web/` and `moltbook/`; do not collapse web's ledger/consent/fetcher.
- Do not delete `benchmark/` — it's a first-class CLI + CI surface.

---

## 9. Corrections to 09_CRITICAL_REVIEW.md

- §3.4: `enable_reasoning_fsm` is no longer dead — it gates llama.cpp `enable_thinking`. `ReasoningRouter` is still dead.
- §4.1: vector-dim validation exists, but **collection creation still hardcodes 768** in two places — "partially resolved" was optimistic.
- New since 09: the slim-offer prompt/grammar drift (§2), the `n_predict: -1` truncation chain (§3), the ignored `log_level` (§4), and the web-console config mirror (§7).

---

## 10. Addendum 2026-07-17 — state machine vs. tool authorization (incident + fix)

Found live via `fcp_core.log.2026-07-17`: `news:today` (and, user-confirmed, `mail:write`) failed in ordinary chat turns with `Tool not authorized in state Reflect`. Root-caused, fixed, and regression-tested the same day. Recorded here because the *class* of bug will recur unless the structural conflation below is resolved.

### 10.1 Root cause **[verified]**

`AgentState` conflates two different concepts:

1. **Turn origin / privilege** — who initiated the work (user chat turn, autonomous agenda/Moltbook cycle, recovery hop). This is what the Gatekeeper allowlists are *for*.
2. **Model-declared loop phase** — the `status` field of the LLM envelope (`Reflect` = "calling tools", `Idle` = "done"). This is conversational protocol. The system prompt *instructs* the model to say `Reflect` on every tool call (status rule 1 in `assembler.rs`), and the missing-status fallback infers `Reflect` whenever `tool_calls` is non-empty (`state.rs:82–96`).

Commit `b02f019` (2026-06-25, the doc-summarize experiment) wired concept 2 into concept 1: four lines in `step.rs` flipped `self.state = AgentState::Reflect` before tool dispatch whenever the model declared Reflect. Consequences of that single flip:

- Every chat-turn tool call was authorized against the **reduced Reflect allowlist** — `news:today`, `web:fetch`, `web:search`, `mail:write`, `calendar:create`, `moltbook:post` etc. died at the Gatekeeper (`gatekeeper.rs` Reflect arm).
- The **per-turn tool-round cap was bypassed** — the `max_tool_rounds` check in `step.rs` explicitly exempts Reflect (that exemption existed *for* the chunk-loop experiment).
- It went unnoticed for ~3 weeks because the commonly-called tools (`memory:query`, `vision:display`, `db:find_connections`) happen to be on the Reflect list.

Textbook drift pattern for this codebase (cf. §2, §5): a feature commit ("teach smoll models to read 1600 chunks") mutated a shared control point (`self.state`) to obtain three side effects (continuation guidance, chunk pruning, cap bypass), and the palette shrink came along invisibly.

### 10.2 Fix applied (same day)

- **Removed the flip** in `step.rs` (§4 "Directive Processing" now carries a NOTE explaining why model-declared `status` must never touch `self.state`). Genuine Reflect entries remain: condensation and `ShiftToReflection`.
- **Retired the `doc-summarize` skill** (embedded default, `suggested_skills` hooks on `doc:*` in `specs.rs`, seeded vault copy). Verdict on the feature itself: an LLM cannot be its own for-loop over thousands of chunks — index drift and early stops are inherent, no prompt fixes this. The `doc:*` tools and RAG path are untouched and work. A v2 must be a **runtime-owned map-reduce** (deterministic chunk iteration in Rust, LLM only for bounded per-chunk work — same architecture as condensation, which works for exactly this reason). Design record: `docs/TODO/HANDOVER-doc-summarize-v1.md`.
- **Regression test:** `core/tests.rs::test_model_declared_reflect_does_not_shrink_chat_tool_palette` — Chat turn, model declares Reflect + calls `news:today`, must execute and end Idle. The test first asserts its own premise (`news:today` ∉ Reflect allowlist), so allowlist changes surface it.

### 10.3 Related finding still open: Recover privilege leak **[verified]**

§5 called the Recover→Chat elevation (`gatekeeper.rs:198–213`, `dispatch_authorization_state`) "fine by design". Qualify that: the elevation promotes Chat-only tools to Chat authorization in **any** Recover round, with no knowledge of who started the turn. An autonomous agenda/Moltbook cycle that fails into recovery can therefore execute `mail:write` / `calendar:create` — the reduced-palette guarantee for autonomous turns silently evaporates under recovery. Not urgent (requires an autonomous turn + tool failure + the model then calling a mutating tool), but it is the same conflation as §10.1 in the opposite direction.

### 10.4 Suggested solution: turn-origin authorization (keeps `AgentState` as-is)

Do not add a fifth state. Split the two concepts:

1. Add a field captured **once at `step()` entry**: `turn_origin: AgentState` (or a dedicated `TurnOrigin { UserChat, Autonomous, … }` enum — the executive already distinguishes these at the call sites in `chat_session.rs` where it sets `state = Chat` before each `step()`).
2. `dispatch_authorization_state` takes `turn_origin` instead of live `self.state`: a turn that began as user chat authorizes as Chat for its entire lifetime (including Recover hops — subsumes today's Recover elevation); a turn that began autonomously keeps its reduced palette **even in Recover** (closes §10.3).
3. Live `self.state` remains what it is good at: UI badge, prompt focus line (`runtime_state_json_contract_focus`), guidance selection (`POST_TOOL_REFLECT_CONTINUATION_GUIDANCE`), condensation bookkeeping.
4. Fold into P1 item 8: when the four `matches!` arms become one data table, make the table's lookup key `(turn_origin, tool)` and delete `dispatch_authorization_state` entirely.

Invariant to enforce from now on (candidate for a CI test): **model output may steer guidance and loop control, but must never write `self.state` or anything the Gatekeeper reads.** The two remaining model-driven writes worth auditing against this rule are in `llm_directive.rs:157,186` (`self.state = Chat` on Task/empty-Reflect before `ShiftToReflection` — benign today, same smell).

### 10.5 Minor note

`Idle` also carries two meanings: "awaiting user input" (UI) and "background-work tool palette" (the deterministic `agenda:complete` executes under Idle authorization, `turn_entry.rs:40`). Harmless today; resolves itself if authorization moves to turn origin.
