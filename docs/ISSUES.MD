suche feedbacks:

- │[zerozero]: Checking your coffee preferences... [running tools…]

- we need to distinct this and maybe move it to status, it should not pollute the main ERIS deck, maybe we can make it like shiny so the user knows there is something cooking
- it messed with ERIS command deck response visibilty
- add more to status in generell / atm command decks i yellw when busy, we need to move this to status

---

-also i node that the command decks BUSY and the status idle a not in sync

---

ctx slider:

egative impacts / risks that still exist
Summaries are lossy
Anything folded into the rolling JSON is compressed by an LLM. You can still lose nuance, exact quotes, formatting instructions, or subtle constraints—even with a strict schema.

Cheap token math ≠ real tokens
Retention uses a rough estimate (chars/4-style). It can be off vs the real model tokenizer → sometimes you keep too much (still tight on context) or fold too much (unnecessary loss).

Big system prompt isn’t what gets “slid”
A lot of context can live in the first system message (identity + tools). Condensation mainly works on the tail. If the prompt + tools are what blow the budget, folding the tail might not fix the pressure; you can still hit limits or get “nothing to fold” behavior while the model is still huge.

Tool-heavy turns can condense more than once per user message
Inside one step(), the loop can call the LLM several times (tool rounds). Each time, if tokens cross the threshold, condensation can run again. That means more summarizer calls, more latency, more cost, and summary-of-summary drift if it happens often in a row.

Ephemeral + stack are two representations
The rolling summary is stored in ephemeral and mirrored on the stack. They’re designed to match after a successful run, but any future bug or partial failure could, in theory, diverge—worth remembering if you debug weird context.

No extra “cooldown” knob (yet)
“Hysteresis” in our discussion mostly meant: no condensation on purely mechanical trims and one summarizer batch per execute_condensation. It does not mean “you can only condense once per minute” or “once per user turn.” If the threshold keeps tripping on successive generations, you can still get repeated condensations.

If you want, we can add a short “Known limitations” note to docs/updated_architecture/ or ISSUES.MD so this stays explicit for future you.

---

Context window, tokens, and layers — design memo (future work)

This document is a snapshot of findings and unresolved questions, not a commitment to implement. Use it after you’ve slept on the problem; revisit when choosing between incremental fixes (calibration, tagging) and larger refactors (layered assembly).

1. Where the codebase stands today (facts)

1.1 Sliding window condensation ([src/orchestrator/context_window.rs](src/orchestrator/context_window.rs))

Implemented: Split stack into head (main system, optional JIT, optional rolling JSON summary) + tail; fold older tail into rolling_summary_v1; persist canonical JSON in ephemeral under fcp:rolling_context_summary; keep a recent tail under a heuristic token budget (chars/4 + small role overhead, see estimate_message_tokens).

Not wired: Ollama’s prompt_eval_count / eval_count are not used inside plan_sliding_condensation or split_tail_fold_and_keep. Calibration (scaling heuristics to last real prompt size) was discussed but not implemented here.

1.2 Condensation trigger ([src/orchestrator/core.rs](src/orchestrator/core.rs))

Still driven after each engine.generate when prompt_tokens + generated_tokens exceeds num_ctx \* condensation_threshold (with web-tool threshold tweak). That uses Ollama-reported totals for the whole request — correct as a tripwire, not as per-message accounting.

1.3 Token metrics channel ([src/engine/token_metrics.rs](src/engine/token_metrics.rs), [src/engine/ollama.rs](src/engine/ollama.rs), [src/executive/router.rs](src/executive/router.rs))

\*\*watch-based last snapshot: LlmTokenSnapshot updated on every successful Ollama completion.

Publisher: OllamaClient only.

Consumer: TUI reads the receiver inside TuiApp::run(..., token_metrics_rx) and passes &LlmTokenSnapshot into [render::draw](src/ui/render.rs) — display only; not connected to context_window.

1.4 Single linear chat_stack

Orchestrator still maintains one Vec<Message> ([Orchestrator::chat_stack](src/orchestrator/core.rs)). Pushes include: system (recovery, tool results, etc.), assistant JSON, user lines from router. No first-class layer tags on Message today.

1.5 TUI / tracing hook you mentioned

User-visible assistant lines go through TuiEvent::IncomingMessage and [UI_RECV_INCOMING_MESSAGE](src/ui/app.rs) tracing when the deck appends a line — parallel to but not structurally tied to how LLM context is built. Useful as a human-facing timeline; it does not solve token accounting.

2. Problem space (detailed)

2.1 Stateless model vs stateful app

Each LLM call is stateless; “memory” is whatever we concatenate into messages. Growth is linear unless we compress (rolling summary), trim, or assemble less.

2.2 One number vs many meanings

Ollama returns one prompt_eval_count for the entire serialized prompt. It does not label:

conversational turns vs tool feedback vs recovery vs system prompt / tool schemas.

So:

Thresholding (condensation trigger) is well-served by that single number.

Retention policy (what to fold first) is not — unless we add structure (tags, layers, or separate buffers).

2.3 Why naive chars/4 in context_window is both useful and wrong

Useful: Cheap, offline, no extra HTTP; enables a relative split (older tail vs recent tail).

Wrong: Systematic drift vs real tokenizer; treats a huge tool blob like a short user line for policy purposes unless we classify messages.

2.4 Tool rounds vs “conversation”

Inside one user step(), multiple generate calls can occur (tool loop). Each produces new prompt_eval_count / snapshot. The “conversation” the human cares about may be one user intent; the stack carries many system lines from tools. Folding “oldest tail” without distinction can summarize away dialogue while tool noise dominates token count — policy mismatch.

2.5 Calibration (future, not done)

Idea: scale heuristic estimates so sum(estimates) ≈ last prompt_eval_count (globally or tail-only). Improves scale; does not encode conversational vs tool semantics without additional structure.

3. “Layers” direction (conceptual)

Idea: Store bounded buffers (Conversational / ToolOrSystem / Recovery — names flexible) and assemble Vec<Message> per turn from a recipe, instead of one ever-growing timeline.

Potential benefits

Independent budgets per layer (caps).

Clearer precedence (what must stay for coherence).

Rust-side efficiency as in explicit ownership and eviction per buffer, not one opaque vec.

How layers complicate things

Coherence: The model needs valid order (user → assistant → tool results → …). Random concat of layers can break causality.

Dependencies: Does the tool layer need “previous tool” context? Often yes for the same multi-step task; the conversational layer needs user goal + commitments, not necessarily every old tool dump if facts are in rolling summary / memory.

Multiple sources of truth: Rolling JSON + conversational deque + ephemeral + vault — risk of contradiction unless precedence is documented (what wins when they disagree).

Phases: “Phase” is not a single enum; it’s position in the inner loop (chat vs tools vs recover). Assembly rules may be state-machine-shaped, not layer-shaped only.

Testing: More surfaces — assembly function must be deterministic and unit-tested.

4. Open questions (explicit)

Q1 (tool vs chat): When trimming, do we always fold older tool/system lines before older user/assistant pairs? What is the minimum conversational tail (e.g. last user + last assistant)?

Q2 (tool-to-tool): Does tool layer B need full visibility of tool A’s output, or only a summary / staged id? (Likely task-dependent.)

Q3 (recovery): Is recovery text always ephemeral after success, or must it feed conversational memory?

Q4 (calibration target): Calibrate whole stack vs tail-only estimate vs per-layer sums?

Q5 (metrics integration): Should context_window read last LlmTokenSnapshot (from watch) only, or also per-generation values passed from step() into condensation? (Watch is last completion — ambiguous in multi-generate turns.)

Q6 (TUI alignment): Should deck lines map 1:1 to “conversational” messages only, or is it purely cosmetic? (Today: cosmetic + tracing.)

5. Suggested sequencing (when you return to implementation)

No order is mandatory — pick depth vs risk.

Calibration in context_window using last known prompt_eval_count + heuristic sum (minimal code, improves scale).

Lightweight tagging at chat_stack.push sites (enum: Conversational | ToolOrSystem | Recovery) — or infer from prefixes where low-risk — then preferential folding (tool first).

Layered assembly — larger refactor: introduce ContextAssembly input struct + single build_messages_for_llm(); migrate pushes gradually.

6. From this assistant: anything to add

Explicit non-goal: Perfect per-token parity with Ollama without a tokenizer or encode API — call it out in any future ADR so scope stays honest.

Single “assembly contract”: If layers land, one module owns ordering + budgets + precedence; avoid scattering rules across orchestrator and router.

Tracing: Consider a single structured event when assembling context (sizes per layer, estimated tokens, optional last Ollama count) — complements UI_RECV_INCOMING_MESSAGE (user-facing) with model-facing observability.

Sleep / creativity: The hard part is policy (what to keep), not data structures; the writeup above is meant to separate those so you can iterate without re-deriving the whole graph each time.

Deliverable

After you accept this plan, the natural next step (separate from “solve it”) is to copy or adapt this content into a repo doc (e.g. [docs/updated_architecture/](docs/updated_architecture/) or [docs/ISSUES.MD](docs/ISSUES.MD) linkage) — only when you ask for Agent mode and a concrete path; this plan file itself is the agreed “writeup” container until then.
