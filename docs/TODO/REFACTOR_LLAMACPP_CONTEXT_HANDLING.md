# Refactor: llama.cpp Long-Context Handling (Transcript Projection)

Status: PARTIALLY IMPLEMENTED (Phase 1 + fold projection landed; 742 tests green, clippy clean)
Owner: (unassigned)

## Implementation log
- **Phase 1 (Role enum) — DONE.** `crate::engine::Message.role` migrated from
  `String` to `enum Role { System, User, Assistant }` (`src/engine/traits.rs`),
  with `PartialEq<&str>` for ergonomic read-site compatibility. All construction
  sites migrated. Ollama/llama.cpp backends map `Role` → their wire role exactly
  as before. Behavior identical; full suite green.
- **Phase 0 probe — DONE.** Live-tested Qwen3.5-9B + Gemma-4-12B on llama-server
  b9571 (`--jinja` default-on) via `/v1/chat/completions` + `/apply-template`.
  Result recorded in §3b: native `tool` role is **unsafe on Gemma** (content
  silently dropped), so fold is the universal strategy.
- **Fold projection (was Phase 3) — DONE.** llama.cpp wire projection is now
  `coalesce_consecutive_roles(normalize_system_messages(raw))` in
  `src/engine/llama_cpp.rs`. `normalize_system_messages` folds stray `system`
  rows into `[System] …` user turns (leading systems merged); the new
  `coalesce_consecutive_roles` merges adjacent same-role turns → strict
  `user`/`assistant` alternation, no content loss (Gemma-safe), and it **subsumes
  the old `merge_trailing_assistant_messages`** (now removed). Backend-derived (no
  manual flag): only the llama.cpp path calls it; Ollama is untouched.
  Unit tests cover alternation, tool-heavy losslessness, and shape idempotency.
- **P1 native `tool` role — DEFERRED** (Gemma-unsafe; optional Qwen-only opt-in later).
- **Not yet done:** Phase 4 (adaptive `n_predict`, prompt-cache prefix stability),
  and the formal Ollama/llama.cpp golden-transcript regression fixtures.

### Soak findings — vault `unknown` + Qwen3.8-27B-UD-Q3_K_XL (2026-08-24)

Config under test: `ngl=99`, `flash_attn=on`, `cache_type_k/v=q8_0`, `num_ctx=32768`,
`n_predict_max=1536`. Log: `vaults/unknown/.fcp/telemetry/logs/fcp_core.log.2026-08-24`.

**Phase A — speed / OOM / multi-tool**
- No JSON protocol failures on short turns. No OOM.
- Decode ~**31–33 tok/s** (was ~4–7 with Q4 + `ngl=40` CPU offload). Confirmed speed win is full GPU offload.
- A3 multi-tool briefing incomplete: `tool_map_offer_cap=5` + ranked subset dropped `wiki`/`vault` from the offer; model correctly used only offered tools (`weather:current`, `news:today`). Not a model crash.

**Phase B — fold / recovery**
- Wire fold active: `coalesced_runs` climbed **2 → 14** across tool-heavy hops. Fold projection doing its job.
- B1: vault searches OK; empty-action recover once (huge thought + null `message_to_user`); then used `web:fetch`/`web:find` (wttr.in) instead of `weather:current` despite weather being re-offered on recover — still reached correct jacket conclusion.
- B2: `memory:query` + `wiki:summary` clean; good synthesis.
- B3: hit **`n_predict_max=1536` hard stop** → `EOF while parsing a string` mid-JSON → `RecoverFromFuckup` → second pass valid (850 toks). Recovery worked. Optional later: raise cap to 2048 (config-only).
- **Condensation did not fire** in A/B. Peak ~10.5k prompt tokens; proactive line ≈ `32768 × 0.5 × 0.8 ≈ 13.1k`. One hop explicitly skipped condensation after parse fail.

**Phase C — condensation** (2026-08-24 continuation) — **PASS**
- Early hops: proactive fired (`stack_est` 13.7k–24k > threshold 13107) but
  `nothing to fold` while tail still fit retain budget (~18k). Expected until older
  prefix exceeded keep window.
- Filler soak (SOAK_FILLER_1/2/3 + short trigger) forced real folds:
  - **3× `fcp.condensation.complete`** before FOLDED reply (pass 1 each; no hard trim)
  - First fold: 6 msgs / 451 tok → rolling JSON 975 chars (`prior_rolling=false`)
  - Second: 10 msgs / 3174 tok → 2323 chars (`prior_rolling=true`, re-fold)
  - Third: 8 msgs / 2537 tok → 1867 chars
  - Fourth (post-probe): 1 msg / 248 tok → 1752 chars
- Model replied `FOLDED` cleanly; probe correctly saw `rolling_summary_v1` system
  JSON and recalled SOAK_FILLER_1 as alpha-batch padding (M1-0001…M1-0126).
- Live process still logged soak-unaware proactive knobs (`ratio=0.8`, threshold 0.5);
  on-disk `0.15`/`0.5` not loaded this session — fold still worked once stack > retain.
- Optional later: visible break line above rolling summary (model already detects
  `rolling_summary_v1`); **knobs restored aligned** in `unknown` config:
  `condensation_threshold=0.55`, `optimize_context_proactive_condensation_ratio=1.0`
  (proactive ≈ post-turn ≈ retain ~18k; needs Eris restart to load).

Audience: humans + AI agents working on Eris/fcp
Scope: architectural refactor of how the conversation transcript is modeled and
handed to each LLM backend. **No behavior change to Ollama intended.** GBNF stays.
llama.cpp remains the recommended backend.

---

## 0. TL;DR

Eris stores the whole conversation in one `Vec<Message>` where every non-user,
non-assistant item — tool results, internal notices, recovery hints, rolling
summaries — is tagged `role: "system"` and interleaved throughout the transcript.

- **Ollama** accepts interleaved `system` messages and passes them through.
- **llama.cpp** uses a strict server-side chat template (Qwen3) that forbids
  `system` anywhere but the very front. To satisfy it, the code rewrites every
  mid-conversation `system` message into a **fake `user` turn** prefixed with
  `[System] …` (`normalize_system_messages` in `src/engine/llama_cpp.rs`).

In long, tool-heavy sessions this produces long runs of consecutive `user`
messages, destroying the `user → assistant` alternation the model was trained on.
The distortion **grows with session length**, which is exactly why llama.cpp
"chokes" late in a session while Ollama stays stable. Three secondary llama.cpp
effects (GBNF masking degraded distributions into confident-but-hollow JSON, the
`n_predict_max` truncation cap, and prompt-cache prefix thrash) compound the
problem but are not the root cause.

**The fix is a modeling change, not a patch:** give internal content an explicit
*semantic* identity, and introduce a **backend-owned transcript-projection seam**
that renders the canonical transcript into each backend's required wire shape.
llama.cpp's projection produces clean, template-legal alternation (tool results
represented as real `tool`/attached turns, not fake user turns), while Ollama's
projection stays essentially 1:1. This is done in phases behind a config flag so
it is safe to land and A/B.

---

## 1. Background: how context flows today (verified against code)

### 1.1 The message model is a stringly-typed 3-role struct

```6:10:src/engine/traits.rs
pub struct Message {
    pub role: String, // "system", "user", "assistant"
    pub content: String,
}
```

There is **no `tool` role and no notion of message origin/kind**. Everything that
is not literally the human or the model is `role: "system"`.

### 1.2 Both backends are stateless; full history is resent every turn

The orchestrator keeps the canonical transcript in `Orchestrator.chat_stack` and,
on every hop, assembles the system prompt, optionally condenses, builds an
LLM-only view, and calls `engine.generate(&view, …)`. Neither backend reuses a
server-side KV cache, slot id, or session token array. This is **identical** for
Ollama and llama.cpp — so context *reuse* is NOT where they diverge.

Dispatch:

```38:52:src/engine/mod.rs
impl LlmEngine for AnyEngine {
    async fn generate(...) -> ... {
        match self {
            Self::Ollama(e) => e.generate(...).await,
            Self::LlamaCpp(e) => e.generate(...).await,
        }
    }
}
```

### 1.3 Internal content is emitted as interleaved `system` rows

Tool results, suppression notices, web caps, recovery hints — all pushed mid-stack
as `role: "system"`. There are ~20+ such sites. Representative:

```124:126:src/orchestrator/core/tool_dispatch.rs
                self.chat_stack.push(crate::engine::Message {
                    role: "system".to_string(),
```

(See also `step.rs`, `transitions.rs`, `prune.rs`, `resolved_tool_recovery/`,
`window.rs`, `helpers.rs`, `turn_entry.rs`.)

### 1.4 Ollama passes `system` through unchanged

```60:65:src/engine/ollama.rs
            let role = match msg.role.as_str() {
                "system" => MessageRole::System,
                "user" => MessageRole::User,
                "assistant" => MessageRole::Assistant,
```

### 1.5 llama.cpp rewrites mid-stack `system` into fake `[System]` user turns

```160:177:src/engine/llama_cpp.rs
    let mut had_stray = false;
    for m in messages.into_iter().skip(leading_system_count) {
        if m.role == "system" {
            had_stray = true;
            out.push(ChatMsg {
                role: "user".to_string(),
                content: format!("[System] {}", m.content),
            });
        } else {
            out.push(m);
        }
    }
```

Plus a second wire-format hack for trailing assistant rows (llama-server rejects
≥2 trailing `assistant` messages):

```186:219:src/engine/llama_cpp.rs
fn merge_trailing_assistant_messages(mut messages: Vec<ChatMsg>) -> Vec<ChatMsg> {
    ...
    // merges consecutive trailing assistant rows into one
}
```

And a condensation-only hack that appends a synthetic `user` line because the
Qwen template raises "No user query found in messages":

```337:348:src/orchestrator/context/window.rs
pub fn ensure_condensation_user_query_tail(stack: &mut Vec<Message>) {
    ...
    stack.push(Message { role: "user".into(), content: "[FCP internal — condensation] ..." });
}
```

These three hacks are **symptoms of the same root cause**: the canonical model has
no vocabulary for "environment/tool/system-directive turns," so each backend
improvises at the wire boundary. Only llama.cpp's improvisation corrupts
alternation, and only at scale.

### 1.6 What actually reaches the wire (worked example)

Canonical stack after a few tool rounds:

```
[system]    <big assembled system prompt + tool defs>
[user]      "summarize my vault notes on X and cross-check the web"
[assistant] {"thought":...,"tool_calls":[{"name":"vault:search",...}]}
[system]    Tool 'vault:search' succeeded: <results>
[assistant] {"thought":...,"tool_calls":[{"name":"web:search",...}]}
[system]    Tool 'web:search' succeeded: <results>
[system]    [SYSTEM] Web tool cap reached (3/turn). ...
[assistant] {"thought":...,"message_to_user":"Here is..."}
```

Ollama wire (roles preserved): `system, user, assistant, system, assistant,
system, system, assistant` — fine for Ollama's template.

llama.cpp wire after `normalize_system_messages`:

```
system:    <assembled system prompt>          # only the leading one stays system
user:      "summarize my vault notes ..."
assistant: {...vault:search...}
user:      [System] Tool 'vault:search' succeeded: <results>
assistant: {...web:search...}
user:      [System] Tool 'web:search' succeeded: <results>
user:      [System] Web tool cap reached ...   # <-- two user turns in a row
assistant: {...final...}
```

Every tool round injects ≥1 fake `user` turn; suppression/cap notices add more,
sometimes back-to-back. Over a long session this is dozens of consecutive/again
`user` turns — far outside the model's training distribution. **This is the choke.**

---

## 2. Root cause statement

> The canonical transcript conflates *semantic role* (who/what produced a turn and
> why) with *wire role* (the 3 labels a chat template accepts). Because "tool
> result" and "system directive" have no first-class representation, they are
> stored as `system`, and each backend must guess how to render them at the wire.
> llama.cpp's strict template forces those guesses into fake `user` turns, and the
> number of such turns grows monotonically with session length — producing
> long-context degradation that is invisible on short prompts and absent on Ollama.

Everything else (GBNF, `n_predict`, prompt-cache) is secondary and additive.

---

## 3. Design goals & non-goals

### Goals
1. **Preserve GBNF** grammar-constrained output on llama.cpp (per-hop subset grammar included).
2. **Keep llama.cpp as the recommended backend**; make it robust at long context.
3. Produce **template-legal, well-formed alternation** for strict templates no
   matter how long / tool-heavy the session gets.
4. **Zero intended behavior change for Ollama**; identical or better output.
5. Consolidate the three scattered wire hacks (`normalize_system_messages`,
   `merge_trailing_assistant_messages`, `ensure_condensation_user_query_tail`)
   into one coherent, tested projection layer.
6. Be **incremental and reversible** (config flag; land in phases; A/B measurable).
7. Obey `.cursorrules`: no `unwrap`/`expect` in prod, no `unsafe`, no blocking the
   tokio runtime, actor/message-passing (no new shared `Arc<Mutex>`), `tracing`
   only, `tempfile` for FS tests.

### Non-goals
- Not introducing server-side KV/slot reuse (out of scope; separate perf effort).
- Not rewriting the orchestrator agent loop or the envelope protocol.
- Not changing the condensation *algorithm* (only where its output is shaped).
- Not changing tool semantics or the `Tool` trait.

---

## 3b. Phase 0 probe results (live, llama-server b9571, `--jinja` default-on)

Ran against real models via `/v1/chat/completions` and `/apply-template`:

- **Model targets confirmed with operator:** Qwen3-class and **Gemma 4 12B** are the
  primary models. Both must be supported.
- **Server tolerance is now high.** Build `b9571` with jinja default-on returns
  HTTP 200 for mid-conversation `system`, consecutive `user` turns, and bare
  `tool` messages on *both* templates. **Structural acceptance ≠ correct
  rendering** — the `normalize_system_messages` re-role hack predates this
  permissiveness.
- **CRITICAL — Gemma silently drops `role:"tool"`.** `/apply-template` on Gemma 4
  renders a `tool` message as *nothing* (the content vanishes from the prompt):

  ```
  tool-role input →  <|turn>user\nread X<turn|><|turn>model\nok<turn|><|turn>user\n?<turn|>   # RESULT dropped
  [System]-user   →  ...<|turn>user\n[System] RESULT_HELLO<turn|><|turn>user\n?<turn|>        # kept, but breaks alternation
  ```

- **Qwen3.5** *does* consume `tool`-role content correctly (answers from it), and
  even accepts a **bare** `tool` message with no `tool_call_id`.

**Design consequence (overrides §4.5 ordering):** native `tool` role (P1) is
**model-dependent and unsafe as a default** — it loses data on Gemma. The
**universal, correct strategy is P2 (fold)**: merge each tool-result /
system-directive into an adjacent conversational turn, coalescing consecutive
internal items, yielding clean `user/assistant` alternation with zero content
loss on **both** Qwen and Gemma. P1 may later be enabled *only* for models proven
to render tool content (capability-gated), but is not the default and not required.

---

## 4. Target architecture

### 4.1 Two-layer model: **canonical semantic transcript** → **backend wire projection**

```
Orchestrator.chat_stack (canonical, semantic)
        │
        │  build_llm_view(...)   # existing slimming/compaction (backend-agnostic)
        ▼
   LLM view (still semantic)
        │
        │  <NEW> TranscriptProjection (backend-owned)
        ▼
   Wire messages[]  ──►  Ollama /api/chat   |   llama.cpp /v1/chat/completions
```

### 4.2 Give messages a semantic identity (without breaking callers)

Add an explicit origin/kind to the canonical message so the projection can make
correct decisions. Two implementation options (pick in Phase 1):

- **Option A (enum role):** replace `role: String` with a typed
  `enum Role { System, User, Assistant, Tool, Directive, Summary }` (+ helpers /
  serde). Cleanest but touches every construction site.
- **Option B (additive tag, lower blast radius):** keep `role: String` but add
  `origin: MessageOrigin` (e.g. `UserInput`, `AssistantProtocol`, `ToolResult`,
  `SystemDirective`, `RollingSummary`, `RecoveryNote`, `MainSystemPrompt`) with a
  default that preserves today's behavior. Construction sites are updated
  incrementally; anything left at default behaves as it does now.

Recommendation: **Option B first** (safe, incremental), with Option A as an
optional later cleanup once all sites carry an origin.

The origin is what the projection reads — NOT the raw string role. This is the key
decoupling.

### 4.3 The projection seam

Introduce a small, pure, **synchronous** function per backend (no I/O, easy to
unit-test), e.g.:

```
// pseudocode – final signature decided in Phase 2
trait TranscriptProjection {
    fn project(&self, view: &[Message]) -> Vec<WireMessage>;
}
```

- `WireMessage` is the backend's on-wire shape (Ollama `ChatMessage`; llama.cpp
  `ChatMsg` + optional `tool_calls`/`tool` fields).
- Ollama projection ≈ current 1:1 mapping.
- llama.cpp projection enforces the **strict-template invariants** below.

Move `normalize_system_messages`, `merge_trailing_assistant_messages`, and the
intent behind `ensure_condensation_user_query_tail` **into the llama.cpp
projection** so all wire-shaping lives in one place with one test suite.

### 4.4 Strict-template invariants the llama.cpp projection must guarantee

1. **Exactly one leading `system`** message (the assembled prompt); everything
   else that is `MainSystemPrompt`/leading gets merged into it (already done).
2. **No `system` after the first turn.** Non-leading system-origin content is
   re-homed, not turned into standalone user turns (see 4.5).
3. **Clean alternation** `user → assistant → user → assistant …` between
   conversational turns. No two consecutive `user` and no two consecutive
   `assistant` conversational turns on the wire.
4. **Tool results are represented as tool turns, not user turns** (see 4.5).
5. **The last turn is a valid query turn** for the template (covers the
   condensation "no user query" case without a bespoke hack).
6. **Deterministic & idempotent:** `project(project(x)) == project(x)` shape-wise;
   pure function; fully unit-testable with golden transcripts.

### 4.5 How to represent tool results on llama.cpp (the crux)

**Updated by the Phase 0 probe (see §3b): fold is the default, not tool-role.**

- **(P2 — DEFAULT) Fold into an adjacent conversational turn.** Merge each
  `ToolResult` / `SystemDirective` / `RecoveryNote` into the **next** `user` turn
  (preferred), or if none follows, append to the **previous** `assistant` turn, as
  clearly-delimited context. Coalesce consecutive internal items into one block.
  No new speaker turn is created, alternation is preserved, and **no content is
  lost on any template** (verified safe on Qwen3.5 and Gemma 4). This is the
  universal path.
- **(P1 — OPTIONAL, capability-gated) Native `tool` role.** Only for models proven
  by `/apply-template` to actually render `tool` content (Qwen: yes; **Gemma: NO —
  content is silently dropped**). Off by default; never used unless a per-model
  capability flag confirms safe rendering.
- **(P3 — legacy) Fake `[System]` user turns.** The current behavior; kept only as
  an internal A/B comparison path, not a target.

Rationale: structural HTTP-200 acceptance is not evidence of correct rendering.
The projection must optimize for what the *template renders*, and folding is the
only representation that is simultaneously alternation-clean and lossless across
the models the operator actually runs.

### 4.6 Where GBNF, `n_predict`, prompt-cache fit

- **GBNF unchanged.** Grammar is attached at the HTTP layer *after* projection
  (`grammar` field). Per-hop subset selection in `step.rs` is untouched. The
  projection only changes `messages[]`, never the grammar.
- **`n_predict_max`:** make adaptive / raise default in Phase 4 (truncation at long
  context turns valid JSON into truncated invalid JSON the grammar can't rescue).
- **Prompt-cache prefix stability:** Phase 4 — keep the leading system prompt +
  early turns byte-stable across hops so llama-server can reuse its cached prefix
  (kills the "stutter"/latency). This is orthogonal but complementary.

---

## 5. Phased implementation plan

Each phase is independently landable and testable. Do NOT merge phases. Respect
`.cursorrules` throughout (no `unwrap`/`expect` outside `#[test]`, `?` + `FcpError`,
`tracing` only, `tempfile` for any FS test, no runtime blocking).

### Phase 0 — Instrumentation & ground truth (no behavior change)
- [ ] Add a debug/trace (or a test-only harness) that dumps the exact wire
  `messages[]` for both backends for a scripted long, tool-heavy session.
- [ ] Capture **golden transcripts** (Ollama wire + llama.cpp wire) as fixtures
  under `src/engine/…/tests` or `docs/`—these become regression anchors.
- [ ] Probe the target Qwen3 GGUF/template: does llama-server accept `role:"tool"`
  and assistant `tool_calls`? Record result in `docs/HOW_TO/LLAMA_CPP_SETUP.md`.
- [ ] Define quantitative long-context metrics (see §7) and record a baseline via
  the existing benchmark harness (`src/benchmark/`).
- Exit criteria: reproducible baseline showing consecutive-`user`-turn growth vs.
  session length on llama.cpp.

### Phase 1 — Semantic message origin (additive, no behavior change)
- [ ] Add `MessageOrigin` (Option B) to `crate::engine::Message` with a default
  that maps to current behavior; update `traits.rs` doc.
- [ ] Thread `origin` through the ~20 push sites (`tool_dispatch.rs`, `step.rs`,
  `transitions.rs`, `window.rs`, `prune.rs`, `resolved_tool_recovery/`,
  `helpers.rs`, `turn_entry.rs`). Tool results → `ToolResult`; caps/suppressions →
  `SystemDirective`; rolling summary → `RollingSummary`; recovery → `RecoveryNote`;
  assembled prompt → `MainSystemPrompt`.
- [ ] No projection yet — wire output must be byte-identical to today (assert with
  Phase 0 goldens). Origin is carried but unused.
- Exit criteria: goldens unchanged; origin populated everywhere; tests green.

### Phase 2 — Introduce the projection seam (behavior-preserving)
- [ ] Define `TranscriptProjection` and `WireMessage` types.
- [ ] Implement `OllamaProjection` (1:1, preserves current mapping) and
  `LlamaCppProjection` that **reproduces today's exact hacks** (move
  `normalize_system_messages` + `merge_trailing_assistant_messages` here verbatim;
  fold in `ensure_condensation_user_query_tail`'s intent as invariant #5).
- [ ] Route `generate()` in both backends through the projection.
- [ ] Delete the now-duplicated inline logic from `llama_cpp.rs::generate` and the
  bespoke condensation tail hack call sites (projection owns it).
- Exit criteria: goldens **still identical**; all three hacks now live behind one
  seam with one test suite. Pure refactor, no output change.

### Phase 3 — Correct llama.cpp projection (the actual fix, flagged)
- [ ] Add config flag, e.g. `llama_cpp.strict_template_projection` (default OFF
  initially) selecting new vs. legacy projection.
- [ ] Implement invariants §4.4 in `LlamaCppProjection`:
  - one leading system; no later system;
  - tool/system-directive origins rendered via P1 (`tool` role) if probe passed,
    else P2 (fold into adjacent conversational turn), else P3 (legacy);
  - coalesce consecutive same-side items;
  - guarantee alternation and a valid trailing query turn.
- [ ] Ensure GBNF attachment and per-hop subset selection are unaffected.
- [ ] Extensive unit tests over golden + synthetic transcripts asserting: zero
  consecutive same-role conversational turns; ≤1 leading system; last turn valid;
  idempotency.
- [ ] Flip flag ON by default for llama.cpp once benchmarks pass (§7).
- Exit criteria: consecutive-`user` growth eliminated; long-context quality metric
  improves vs. Phase 0 baseline; Ollama untouched.

### Phase 4 — Secondary long-context hardening (complementary)
- [ ] `n_predict_max`: raise default and/or make adaptive to remaining ctx so long
  replies aren't truncated mid-envelope (`llama_cpp.rs` §334–343; config
  `default_llamacpp_n_predict_max`, currently `2048`).
- [ ] Prompt-cache prefix stability: keep leading system prompt + earliest turns
  byte-stable across hops (investigate `upsert_system_prompt` churn in `step.rs`
  and condensation head rewrites) so llama-server reuses cached prefix → removes
  the "stutter".
- [ ] Consider disabling/relaxing GBNF on pure recovery re-asks so a confused
  long-context model can fail loudly and be re-prompted (mirror the summarizer's
  `attach_session_grammar: false`).
- Exit criteria: latency ("stutter") and truncation reduced; measured.

### Phase 5 — Validation, docs, rollout
- [ ] Full benchmark sweep (both backends, short + long sessions).
- [ ] Update `docs/updated_architecture/03_ENGINE_LLM_AND_ROUTING.md` and
  `02_ORCHESTRATOR_LAYER.md` with the projection seam.
- [ ] Update this file's status; write a short HANDOVER note.
- [ ] Remove legacy projection path + flag after a soak period (optional).

---

## 6. Files likely touched (map for agents)

| Area | File(s) | Role in refactor |
| --- | --- | --- |
| Message model | `src/engine/traits.rs` | add `MessageOrigin` (Phase 1) |
| Backend dispatch | `src/engine/mod.rs` | route through projection |
| llama.cpp backend | `src/engine/llama_cpp.rs` | move hacks → projection; `n_predict` |
| Ollama backend | `src/engine/ollama.rs` | 1:1 projection (no behavior change) |
| New projection | `src/engine/projection/` (new module) | the seam + per-backend impls |
| Tool result pushes | `src/orchestrator/core/tool_dispatch.rs` | set `origin` |
| Step/loop pushes | `src/orchestrator/core/step.rs`, `transitions.rs`, `turn_entry.rs`, `helpers.rs` | set `origin` |
| Condensation | `src/orchestrator/context/window.rs` | drop bespoke user-tail hack; set summary origin |
| Pruning/recovery | `src/orchestrator/context/prune.rs`, `resolved_tool_recovery/` | set `origin` |
| LLM view | `src/orchestrator/context/view.rs` | unchanged logic; stays pre-projection |
| Config | `src/config.rs` | `strict_template_projection` flag; `n_predict_max` default |
| Setup docs | `docs/HOW_TO/LLAMA_CPP_SETUP.md` | record template `tool`-role capability |

---

## 7. How we measure success (long-context quality)

Baseline in Phase 0, re-measure in Phase 3/5. Suggested metrics:

- **Structural:** max & mean run-length of consecutive same-role conversational
  turns on the llama.cpp wire, as a function of turn index. Target: ≤1 (perfect
  alternation) regardless of session length.
- **Quality:** task-success / answer-quality on a scripted long tool-heavy session
  (use `src/benchmark/`), llama.cpp new vs. legacy vs. Ollama.
- **Protocol health:** envelope parse-failure rate and recovery re-ask count late
  in session (should drop).
- **Latency:** time-to-first-token per hop (prompt-cache thrash indicator) before
  vs. after Phase 4.
- **Truncation:** fraction of responses hitting `n_predict_max` before vs. after.

A/B is enabled by the Phase 3 config flag on identical scripted inputs.

---

## 8. Risks & mitigations

- **Template `tool`-role support varies by GGUF.** → Phase 0 probe + P1/P2/P3
  fallback ladder; never assume.
- **Silent behavior change for Ollama.** → Phase 1/2 assert byte-identical goldens;
  Ollama projection is 1:1; flag only affects llama.cpp.
- **GBNF regression.** → grammar attaches post-projection; add a test asserting the
  `grammar` field + subset selection are unchanged by projection.
- **Condensation interactions.** → route condensation output through the same
  projection; keep summarizer's `attach_session_grammar: false`; retire the
  ad-hoc `ensure_condensation_user_query_tail` in favor of invariant #5.
- **Large blast radius from origin threading.** → Option B (additive tag with
  default) keeps unconverted sites behaving exactly as today.
- **`.cursorrules` violations.** → projection functions are pure/sync (no runtime
  blocking, no `unwrap`), use `?`/`FcpError`, `tracing` for diagnostics, `tempfile`
  in any FS-touching test.

---

## 9. Open questions (resolve during Phase 0/1)

1. Does the target Qwen3 GGUF template reliably accept `role:"tool"` +
   assistant `tool_calls` via llama-server `/v1/chat/completions`? (Decides P1 vs P2.)
2. Enum role (Option A) vs additive origin (Option B) — confirm Option B first.
3. Should the projection live in `src/engine/projection/` or inside each backend
   module? (Proposal: shared module + per-backend impls for testability.)
4. Do any Ollama-template edge cases rely on interleaved `system` semantics we'd
   want to preserve intentionally? (Verify with Ollama goldens.)
5. Should `build_llm_view` eventually become origin-aware (e.g. tool-result hints
   keyed by origin rather than string parsing in `stack_lines`)? (Later cleanup.)

---

## 10. Glossary

- **Canonical transcript / `chat_stack`:** the single source-of-truth message list
  the orchestrator maintains.
- **LLM view:** a slimmed, backend-agnostic copy produced by `build_llm_view`
  (compaction, snippet trimming) — still semantic, pre-wire.
- **Projection:** NEW backend-owned step turning the LLM view into the exact
  `messages[]` a given server/template accepts.
- **Wire role vs semantic role:** wire role = the 3 labels a template accepts;
  semantic role/origin = what actually produced the turn and why.
- **Strict template:** a chat template (e.g. Qwen3) that only allows `system` at
  the front and expects clean user/assistant (and optionally `tool`) alternation.
- **GBNF subset grammar:** per-hop grammar aligned to the tools visible that hop
  (`GbnfSubsetCache`, `step.rs`) — unaffected by this refactor.
```
