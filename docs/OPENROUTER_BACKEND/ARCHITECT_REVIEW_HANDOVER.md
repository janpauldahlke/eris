# Architect review handover — `feature/openrouter`

**Pass 2:** after [REVIEW_MD](REVIEW_MD), implementation landed C1/C4/C2/A. Delta brief: [ARCHITECT_REVIEW_HANDOVER_2.md](ARCHITECT_REVIEW_HANDOVER_2.md). This file is the original pass-1 brief; do not treat it as the current worktree.

**To:** reviewing architect (Opus 4.8 or equivalent)
**From:** implementation session on native OpenRouter tools + GBNF regression
**Date:** 2026-09-15
**Branch:** `feature/openrouter` (tracks `origin/feature/openrouter`)
**Ask:** review the **whole branch vs `main`**, not only the last smoke fixes. Operator wants an architectural read: is the native-tools projection sound, did we break local backends / condensation, what should land vs wait.

**Operator stance:** smoke was “ookish enough” to call review. Implementation agrees — **review now, do not stamp merge-ready**.

---

## 1. Verdict on testing (read this first)

Do **not** object to this review. Object to treating the branch as done.

| What smoke proved | What it did not prove |
|---|---|
| Native OpenRouter path can greet, call tools, and **emit a user-facing answer** after tools (task → perform → report). | Hosted Auto will stop tooling without dummy/duplicate calls. |
| Parallel native `tool_calls` assemble over SSE (no more concatenated `vault:searchvault:list`). | Condensation pairing for native `assistant.tool_calls` + `role:tool` frames (billy `num_ctx` is huge; it never folded). |
| llama.cpp GBNF on vault `unknown` still does the old envelope loop and still reports. | Native frames do not pollute rolling condensation / LLM-view placeholder / recovery strip. |
| Phase 0 schema lowerer: `memory:query` etc. get real args, not `{}`. | LiteLLM / Qwen will not invent junk names, string-typed ints, or empty `memory:stage`. |

Call review because the remaining questions are **architectural** (speech-act split, stack format, condensation), not “one more Lisbon prompt.”

Do **not** run the full local `cargo test` suite on this host (hybrid-GPU GNOME session-drop). See `docs/TODO/SOFTEN_TEST_FULL_OOM.md`. `cargo check` / targeted tests only.

---

## 2. What this branch actually is

`main` does **not** have OpenRouter. Unique history is two stacked jobs:

1. **Original OpenRouter backend** (`740a224` and the metaplan): third `LlmEngine`, Bearer key from env, embeddings stay local, first structured path was envelope `response_format` (`json_schema` → `json_object` → prompt-only).
2. **Native tools migration** (`1f1ab95` … `7289299` + follow-up): stop asking the hosted model to author `{thought,status,message_to_user,tool_calls}` JSON. Send OpenAI `tools` / `tool_choice`. Project native channels back into the **same** orchestrator `LlmResponse`.

Plans:

- [0_METAPLAN.md](0_METAPLAN.md) — adding the backend at all (still the privacy / fail-open / embed-decouple contract).
- [OPENROUTER_NATIVE_TOOLS_MIGRATION.md](OPENROUTER_NATIVE_TOOLS_MIGRATION.md) — phases 0–5. Marked DONE in-doc. **Several non-goals were later violated on purpose** (see §4).

Uncommitted on the worktree (must be in the review, not lost):

- `src/executive/peripherals.rs` — `sanitize_llama_server_child_env`: strip `LLAMA_API_KEY` / `LLAMA_API_KEY_FILE` from managed llama-server children. Operator `.zshrc` exports `LLAMA_API_KEY=local` for llama-dsh; llama-server treats that as `--api-key`; Eris sends no Bearer → HTTP 401 “Invalid API Key”. Health 200, chat 401. **Do not tell the operator to export a llama API key for Eris.**

---

## 3. As-built architecture (ignore the migration non-goals)

### Engine boundary

`OpenRouterClient` talks OpenAI `/v1/chat/completions` (here: LiteLLM `https://llmproxy-dev.akqui.net/v1`, model id `qwen3.6:27b`).

**Label trap:** that id is **not** a Qwen 3.6 27B. Host says it is the same **Qwen 3.8 27B** family as local `Qwen3.8-27B-UD-Q3_K_XL.gguf`. Q3 vs Q8 and “3.6 vs 3.8” do **not** explain the behavior gap. The harness does.

When tools are offered:

- attach `tools[]` from the **same** offered names as the JSON-schema subset (`JsonSchemaSubsetCache` / Phase 0 `schema_to_openai`);
- **omit** envelope `response_format` (mutually exclusive);
- `tool_choice = auto` (see below);
- SSE Solution B: assemble `delta.tool_calls[]` fragments by index.

HTTP 400 with tools attached → session downgrade to envelope `response_format`, retry once.

Projection: `llm_response_from_engine` in `src/orchestrator/llm_support/json_envelope.rs`.

| Native | Envelope |
|---|---|
| `message.tool_calls[]` | `LlmResponse.tool_calls`, status Reflect |
| `message.content` (non-JSON talk) | `from_native_talk` → `message_to_user`, Idle |
| `message.reasoning` | `thought` (never content) |
| envelope JSON (local / downgrade) | parse as before |

`EngineToolCall.id` is stabilized (`call_{i}` if missing). Orchestrator `ToolCall.provider_call_id` is the OpenAI `tool_call_id`. Local backends still return empty `tool_calls` and fold results as system text.

### Orchestrator (this is the real review surface)

Phase 3 **did** change the chat stack for OpenRouter:

- assistant hop with `Message.tool_calls`;
- results as `Role::Tool` + `tool_call_id` (`openai_wire.rs` emits `role:"tool"`; consecutive tool frames are **not** coalesced);
- llama.cpp / Ollama unchanged fold path.

Post-smoke (commit `7289299` and tests in `src/orchestrator/core/tests.rs`):

- Deck emit used to require envelope JSON. Native Idle talk was silent. Now `emit_optional_user_message_from_engine` projects via `llm_response_from_engine`. Combined tool+prose hops emit. Weather stitch appends to native prose.
- `step.rs` Halt / IncomingMessage uses the full `EngineResponse` (content + tool_calls + reasoning), not JSON-only parse.

`tool_choice`: **`Auto` while tools are offered**, through `max_tool_rounds`, then omit `tools[]` for a final talk pass. **`Required` is illegal for the summarize hop** (that was the silent-after-tools bug when combined with JSON-only deck emit). Do **not** put `tool_choice` in `config.toml` in this review unless you argue it belongs; operator deferred the knob.

### GBNF speech acts vs native speech acts

This is the load-bearing insight from smoke.

llama.cpp GBNF: **every hop is one envelope**. Tool and talk are both legal in that object. After tools, the next hop still must be valid FCP JSON, so the model fills `message_to_user`. That reads as “smart, finishes the job.”

Native Auto: **tool hop and talk hop are different API shapes**. After `role:tool` results, the model may call again. Duplicate-suppress (still on) then forces a talk pass. Dummy `memory:stage`, junk names (`\n</parameter`), `top_k: "1"` are this loop, not a worse model.

Do **not** copy PR #23’s deletion of duplicate-suppression. Operator explicit. Per-turn fingerprint + non-repeatable tools stay.

### Config knobs added on this work

- `slim_tool_description_preview_chars` (default 120, `0` = no truncation). Phrase-map description column, **not** `tool_map_offer_cap` / JIT top-k. Billy set to **400**. Documented in `docs/HOW_TO/ADDING_A_TOOL.md`.

Billy (OpenRouter smoke): slim + JIT on, offer cap 5, jit_top_k 5, google/moltbook/vision/audio/discord off, `max_tool_rounds` 40, `max_tokens` 4096, generation timeout 300s.

Unknown (GBNF regression): LlamaCpp, 32k ctx, proactive condensation on, google/vision on, moltbook off, Identity.md ~2k chars (Rune). ToolRouter online. `native_tool_calls=0`, `grammar_attached=true` every hop.

---

## 4. Plan vs reality (non-goals that did not hold)

Migration §5 said: no change to orchestrator state machine, **chat-stack format**, **context view/condensation**, presentation.

Reality:

- chat stack gained `Role::Tool` + assistant `tool_calls` (required for a real OpenAI round-trip);
- deck / step emit changed (required or native talk is invisible);
- view / prune / last-tool / recovery were taught about `Role::Tool` in dispatch;
- condensation was **not** fully updated (see §6).

That is the main architectural question: **was Option A (`Role::Tool` on the stack) the right split, and did we finish the consumers?**

---

## 5. Smoke evidence (2026-09-15)

Logs:

- Billy: `vaults/billy/.fcp/telemetry/logs/fcp_core.log.2026-09-15` (native; look ~2093–4140 for the diverse protocol after the deck fix).
- Unknown: `vaults/unknown/.fcp/telemetry/logs/fcp_core.log.2026-09-15` (GBNF; ignore 16:45–16:50 401 sessions; real run **16:52+**).

### Billy (native) — after deck fix

Every turn got `UI_EMIT_INCOMING_MESSAGE`. Parallel vault search+list and Kyoto weather+wiki **reported**. Remaining tic: Auto keep-tooling.

Seen in-session:

- duplicate suppress (clock, wiki) → forced talk;
- dummy `memory:stage` (often missing tags / noop);
- Gatekeeper schema rejects (`top_k` as string, junk tool name);
- `web:fetch` invented `mission_id` then recovered;
- DB REST timeout (infra, not protocol);
- Identity search quoted the wrong note (`mail-recipient-verify.md`).

Billy never hit rolling condensation (context budget too large).

### Unknown (GBNF) — 16:52 session

Clean protocol smokes: greet/no-tools, talk-only memory question **without tools**, health, clock once (no duplicate), vault search, list, Lisbon, Hanseatic wiki, parallel Hagbard+list, Kyoto weather+wiki **in one envelope**, news, `ready` Idle first hop.

Freeform “impress me” turn: 8-tool batch, `memory:stage` missing `tags` (recoverable), then GBNF JSON **truncated at 2048 completion tokens** (`EOF while parsing`). Grammar does not save a cut-off object.

Condensation **did** fire (~17:07): `fcp.condensation.proactive` for a while, then `tail_plan` folded 4 early msgs, `rolling_summary_v1` 778 chars, kept 97, no hard trim, `strip_recovery` removed system rows. **No `Role::Tool` frames in that stack**, so the native pollution path was **not** exercised.

### Why local “felt smarter”

Not quant, not 130k, not a different Qwen minor version.

1. GBNF forces the report hop.
2. Unknown Identity + prefetch actually inject persona and vault notes (billy identity is a stub; billy vault is thin). Prefetch is hundreds of chars, not the context window.
3. Subset GBNF types args; hosted Auto + JSON schemas still admit junk that Gatekeeper then rejects.

Memory injection explains **voice**. It does not explain Kyoto/clock discipline.

---

## 6. Review targets (please answer these)

### A. Native projection and Auto

- Is `llm_response_from_engine` the right seam, or should native talk/tools stay engine-private?
- Is `tool_choice=auto` + tools-until-`max_tool_rounds` the correct GBNF analogue, or do we need a post-tool “must talk” policy (without `Required` on the first hop)?
- Dummy `memory:stage` after successful tools: prompt, Auto, or missing “you already have results, speak” constraint?

### B. Chat stack / wire

- Confirm `Role::Tool` + `provider_call_id` round-trip in `openai_wire.rs` / `tool_dispatch.rs` cannot leak onto llama.cpp/Ollama hops.
- Consecutive tool frames not coalesced — still correct for LiteLLM?

### C. Condensation (analysis only; not implemented)

Native frames are cloned into the fold. Risks we already named:

1. **Unpaired split:** folding may keep `assistant.tool_calls` and drop the matching `role:tool` (or the reverse). GBNF folds one JSON blob; native is two message kinds.
2. **`estimate_message_tokens`** (`src/orchestrator/context/window.rs`) counts **content only**. Native assistant hops often have empty content and fat `tool_calls`. Proactive threshold will lie on OpenRouter long tools.
3. **`strip_recovery_system_rows_after_head`** only removes `role == "system"`. Native recovery/duplicates sitting on `Role::Tool` will not strip.
4. **LLM view** `assistant_non_json_placeholder` (`view.rs`): later hops replace non-JSON assistant **content** with an omit marker. Native talk lives in content; later hops can **blank the report the user already saw** from the model’s view. GBNF reports stay in envelope JSON and survive.

Unknown proved the GBNF fold path still runs. It does **not** prove C1–C4.

### D. Local invariance

- GBNF regression on unknown looks good for the protocol list.
- 2048-token GBNF truncate on large stacks: existing llama.cpp limit, not introduced here — still a product hole if OpenRouter review proposes larger `max_tokens` without a local cap story.
- Uncommitted API-key sanitize: is env-strip the right fix vs Eris sending Bearer when the child has a key?

### E. Scope vs `main`

`git diff --stat main...HEAD` is large (~153 files). Routing/web/skills/UI noise may be from `3d3fb5a merge main` plus native work. Please separate:

- must-review: `src/engine/openrouter.rs`, `openai_wire.rs`, `structured/schema_to_openai.rs`, `orchestrator/llm_support/json_envelope.rs`, `core/{step,deck,tool_dispatch,condensation}.rs`, `context/{view,window,assembler}.rs`, `traits.rs`, `tools/system/health.rs`;
- confirm-unrelated: web allowlist, vision, audio, moltbook, skills store, etc.

### F. Explicit non-actions

- Do **not** delete duplicate-suppress (PR #23).
- Do **not** add `tool_choice` to config unless you recommend it as a follow-up.
- Do **not** `git add` / commit (workspace rule: human only).
- Do **not** “fix” local Q3 vs hosted “dumber” by copying unknown Identity into billy unless the operator asks. That is vault content, not a protocol bug.

---

## 7. Absolute constraints (still in force)

`.cursorrules`: no `unwrap`/`expect` outside tests, no `unsafe`, no `println!`, no `Arc<Mutex<T>>` for shared mut, `tempfile` for test FS, tracing only, no git autonomy.

API key never in logs, health, or serialized config (health reports `native_tools | json_schema | json_object | off`).

---

## 8. Suggested reading order

1. This file.
2. [OPENROUTER_NATIVE_TOOLS_MIGRATION.md](OPENROUTER_NATIVE_TOOLS_MIGRATION.md) §§1–5 and §8–9 (then treat non-goals as stale).
3. `src/orchestrator/llm_support/json_envelope.rs` — `llm_response_from_engine`, `from_native_talk`, `from_native_tool_calls`, id stabilize.
4. `src/orchestrator/core/step.rs` — `ToolChoice::Auto`, Halt emit.
5. `src/orchestrator/core/deck.rs` — emit from engine.
6. `src/orchestrator/core/tool_dispatch.rs` — `Role::Tool` results + duplicate suppress.
7. `src/engine/openai_wire.rs` + `src/engine/openrouter.rs` SSE assembler.
8. `src/orchestrator/context/view.rs` placeholder + `window.rs` token estimate + `core/condensation.rs` strip.
9. Billy log 2093–4140 and unknown log 16:52–17:08.

Tests worth skimming: `src/orchestrator/core/tests.rs` (native talk/tools/invalid JSON/weather stitch), `src/engine/openai_wire.rs` / openrouter wiremock, `schema_to_openai` Phase 0 cases, `gbnf_and_json_schema_subsets_offer_identical_tools`.

---

## 9. What we want back from you

Not a rewrite. A short architecture note:

1. **Ship / wait / split:** can native tools merge with condensation follow-ups, or is C blocking?
2. **Speech-act policy:** keep Auto as-is, or add an explicit post-tool talk constraint (engine, prompt, or orchestrator)?
3. **Stack format:** keep `Role::Tool` or is the dual-path (native vs system-fold) going to keep leaking into view/condense/recovery forever?
4. Concrete defects you would fix **before** more smoke, vs accept as hosted-model tics.

Operator will keep iterating; this is the pause to see if the shape is right.
