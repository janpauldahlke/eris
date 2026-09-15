# Architect review handover 2 — post-REVIEW_MD implementation

**To:** same architect as [REVIEW_MD](REVIEW_MD)
**From:** implementer after landing the “before more smoke” list
**Date:** 2026-09-15 (same day as pass 1)
**Branch:** `feature/openrouter`
**Ask:** delta review of the consumer fixes you prescribed. Do **not** re-litigate the native-tools slice (`openrouter.rs` / `openai_wire.rs` / projection). You already called that sound.

**Prior artifacts (read in this order):**

1. [ARCHITECT_REVIEW_HANDOVER.md](ARCHITECT_REVIEW_HANDOVER.md) — original brief (still accurate on engine/wire/speech-acts).
2. [REVIEW_MD](REVIEW_MD) — your verdict: review-not-merge; C1–C4 in rewriters; Auto + post-tool talk; keep `Role::Tool`.
3. This file — what we implemented, second smoke, remaining holes.

**Operator stance:** second smoke after C1/C4/C2/A looked better on hosted Q8. Operator then asked whether to call you again. Implementer: **yes, a short delta review**, not a full branch re-read. Still **do not stamp merge-ready**.

Do **not** run the full `cargo test` suite on this host (`docs/TODO/SOFTEN_TEST_FULL_OOM.md`). Targeted tests only. Do **not** commit (human only). Do **not** copy PR #23. Do **not** add `tool_choice` to config unless you reopen that.

---

## 1. Why re-review now

Pass 1 said: native core is mergeable; **block on C1–C4**; add post-tool talk (A); C3 and GBNF-2048 can wait.

We implemented C1, C4, C2, and A **uncommitted** on the worktree. Then we ran another natural-language smoke on both vaults. Then we found a **config** hole (billy embeddings), not a protocol hole.

This pass should answer: **did we implement your prescription correctly, and is anything still blocking?**

---

## 2. What landed (your fix order)

All of this is **uncommitted** (`git diff` vs `HEAD`). Pass-1 native-tools commits are already on the branch.

| Id | Status | Where |
|---|---|---|
| C1 atomic native-round fold/trim | done | `src/orchestrator/context/window.rs` |
| C4 stop blanking native talk | done (narrower than “backend flag”) | `src/orchestrator/context/view.rs` |
| C2 count `tool_calls` + `tool_call_id` | done | `window.rs` `estimate_message_tokens` |
| A post-tool talk constraint | done, OpenRouter Chat only | `tool_batch.rs`, `tool_dispatch.rs`, `step.rs`, `post_tool_guidance.rs` |
| C3 strip `Role::Tool` recovery | **not done** (you said follow-up) | — |
| GBNF 2048 truncate | **not done** (you said orthogonal) | — |

`sanitize_llama_server_child_env` from pass 1 is now **committed** as `f7dd5e0` (no longer uncommitted).

### C1

Shared predicate: `is_native_tool_call_assistant` / `is_native_tool_result` / `is_native_tool_round_row`.

`snap_split_at_native_rounds` moves the fold/keep cut so an assistant-with-`tool_calls` and its contiguous following `role:tool` frames stay on one side. Trim uses `native_round_drop_count` so the oldest tail drop cannot orphan a round.

**Please check:** orphan `role:tool` with no preceding assistant (broken stack already) — we walk back to a preceding assistant if present; if the tail *starts* on `role:tool`, we drop the contiguous tool results together. Is that the right snap when the assistant already folded away?

Billy still has `num_ctx = 1_000_000` and `optimize_context_proactive_condensation = false`, so C1 is **still unexercised in hosted smoke**. Unknown GBNF fold does not hit native frames.

### C4

You suggested: exempt native talk, keyed off backend or a talk-turn marker.

We did **not** key off backend. Omit-placeholder now fires only when protocol parse fails **and** content `trim_start()`s with `{`. Bare native prose is kept. Broken envelope JSON still gets the omit marker. Envelope successes still compact.

**Please check:** is the `{` heuristic enough, or do you still want an explicit talk-turn marker? A native hop that starts with `{` for some other reason would still be blanked.

### C2

`estimate_message_tokens` adds serialized `tool_calls` JSON (fallback: name + arguments + id chars) and `tool_call_id`. Same `/4 + 1 + 4` overhead as before.

### A — post-tool talk

New `ToolBatchDecision::PostToolTalkPass { message }`. After a **successful OpenRouter Chat** batch (including weather-only), dispatch returns this instead of `Continue`. `step.rs` handles it like `SuppressOnlyIdlePass`: `state = Chat`, push guidance, **`tools_needed = false`**, clear targeted schemas.

`should_force_post_tool_talk` = `is_openrouter() && Chat`. **Reflect still `Continue`s** with the old continuation guidance. GBNF/llama.cpp/Ollama unchanged.

Guidance constant `POST_TOOL_TALK_NOW_GUIDANCE` still speaks envelope (`status Idle`, `message_to_user`, `tool_calls []`). That matches the hop that follows: when `tools_needed` is false, OpenRouter attaches **envelope `response_format`** with an empty-tool subset and **`native_tools = None`** (`step.rs` ~476–484). So the answer hop is **not** native Auto — it is the same “omit tools, force JSON talk” shape as llama.cpp’s empty-tool GBNF.

**Please check this interaction.** Is envelope-on-the-answer-hop what you wanted, or should the talk pass stay native (`tools` omitted, no `response_format`, bare prose)? We followed “force one no-tool pass”; we did **not** keep native talk on that hop.

Test: `openrouter_chat_success_forces_talk_pass` in `tool_dispatch.rs`.

---

## 3. Second smoke (after C1/C4/C2/A)

Logs (same files, later slices):

- Unknown GBNF: `vaults/unknown/.fcp/telemetry/logs/fcp_core.log.2026-09-15` ~2462–4081
- Billy native: `vaults/billy/.fcp/telemetry/logs/fcp_core.log.2026-09-15` ~4140–4934

Operator report: hosted Q8 now **felt smarter** than the first native smokes (health / wiki / fetch; recalled Bergen without re-calling weather). That matches your diagnosis: C4 was live in billy, and Auto-keep-tooling was the dummy-`memory:stage` loop. A is the intended brake.

Unknown GBNF still looked clean. C1 pairing still not hit on billy (1M ctx).

### ToolRouter / embeddings (config, not branch)

Billy ToolRouter was **offline** the whole first native campaign: `embed_backend = "Ollama"`, `nomic-embed-text` never pulled → `ROUTER_UNAVAILABLE` → full 37-tool phrase map, prefetch fail, and often `tools_attached=false`.

That last bit is a **real code hole**, not just missing nomic:

`step.rs` slim OpenRouter branch, `offered.is_empty()` → `(None, None, None)` — **no native `tools[]`**. Llama.cpp’s matching empty-offer branch still attaches **session GBNF**. So router-down on OpenRouter silently drops native tools; Recover was one of the few hops that still attached them.

We did **not** fix that empty-offer attach. Operator pivoted: **start llama-embed like vault `unknown`**, do not assume Ollama nomic, do not invent a remote embed URL tonight.

Operator vault change (not in git if `vaults/` is ignored): billy now `embed_backend = "LlamaCpp"` with the same GGUF / `127.0.0.1:8091` as unknown. Chat stays OpenRouter. `ensure_llama_embed_server` already existed on OpenRouter+LlamaCpp embed. Qdrant still only stores vectors Eris produces (nomic 768-d). Document RAG / reindex / ToolRouter all share that one embedder.

**Please check:** empty slim `offered` on OpenRouter should attach the **full allowed roster** (llama.cpp analogue), not omit `tools[]`. We think that is the remaining correctness fault if embed/router is down again. Not implemented.

---

## 4. What we want back (short)

1. **C1/C4/C2/A:** accept, nits, or rework? Especially C4 `{` heuristic vs marker, and A’s envelope answer hop vs native prose.
2. **Still blocking merge?** Your pass-1 block was C1–C4. If the implementations are right, is the remaining block only “exercise C1 on a real-ctx OpenRouter vault” plus the empty-offer attach?
3. **Empty slim offer:** fix before more hosted smoke, or accept as long as llama-embed is up?
4. **C3 / GBNF 2048:** still follow-ups?
5. **Non-actions still in force** unless you reopen them: no PR #23, no `tool_choice` config, no Identity copy from unknown → billy, no remote embed URL in this slice.

Concrete file list for this delta:

- `src/orchestrator/context/window.rs` (C1 + C2 + tests)
- `src/orchestrator/context/view.rs` (C4)
- `src/orchestrator/loop/tool_batch.rs`
- `src/orchestrator/core/tool_dispatch.rs` (A + test)
- `src/orchestrator/core/step.rs` (`PostToolTalkPass` + the empty-offer `(None, None, None)` branch)
- `src/orchestrator/llm_support/post_tool_guidance.rs` (`POST_TOOL_TALK_NOW_GUIDANCE`)

Net: you said the shape is right. We built the consumer slice you asked for. This review is “did we build it,” plus the empty-offer leak that smoke revealed once ToolRouter was actually looked at.
