suche feedbacks:

- │[zerozero]: Checking your coffee preferences... [running tools…]

- we need to distinct this and maybe move it to status, it should not pollute the main ERIS deck, maybe we can make it like shiny so the user knows there is something cooking
- it messed with ERIS command deck response visibilty
- add more to status in generell / atm command decks i yellw when busy, we need to move this to status

---

@fcp_core.log.2026-04-01 (4765-4928)

---

What the log shows
Boot / ingest (4765–4770)
Qdrant sidecar comes up, semantic brain online, Boot-time vault ingestion complete files=2 — ingest path ran and counted two markdown files. Nothing here indicates an ingest bug.
The real failure: wrong memory:query JSON (4816–4845 and again 4871–4901)
The model calls:

{"name": "memory:query", "args": {"staged_id": "", "tags": ["user", "preference", "coffee"]}}
Your tool only accepts query (required) and optional filter_tag. There is no staged_id or tags array on memory:query.

So the gatekeeper correctly rejects it:

"query" is a required property
memory:query never runs — no embedding, no Qdrant, no fallback, no WARNING line.
So this session does not test your improved ingest or search_memory_query at all for that turn.

Why the model might emit that shape
It is mixing up another memory tool’s mental model (e.g. staging / tag lists) with memory:query. PRELLM_SEMANTIC_EMPTY on turn 2 (4817–4818) means the router did not promote memory:query to Tier‑1 first; the model saw the full roster and still picked the wrong argument schema.

Worse: recovery produces false “memory” (4858–4869, 4914–4920)
After the schema error, the retry round returns Idle with a detailed coffee preference (cinnamon, single espresso, medium cup). That cannot come from your vault file (strong, white, sugar) and cannot come from tool output, because the tool never executed. So this is plain hallucination, framed as if memory was checked.

The same bad args repeat on turn 3 (4889–4900), so the failure mode is stable, not a one-off.

Problem-space summary
Layer Status
Ingest Looks fine (files=2).
memory:query implementation Not exercised; calls never passed validation.
Model / prompt Systematically wrong args (staged_id / tags vs query / filter_tag).
Recovery Lets the model answer without a successful tool read → dangerous for “remember” questions.
Directions that actually address this (for later work)
Prompt / JIT / examples — Make the only valid memory:query shape unmistakable (query string first; filter_tag optional string), and contrast explicitly with tools that use tags arrays.
Router — Coffee / “remember” style utterances should be more likely to hit memory:query in Tier‑1 so the right schema is in front of the model earlier.
Recovery policy — If memory:query failed schema validation, avoid accepting an Idle answer that claims recall until a valid call succeeds or the model says it could not query.

### Regression hypothesis: commit `124d0ea` (S08 — pre-LLM routing / “telemetry”)

That change was **not** logging-only. It introduced **two-tier tool prompts**: Tier 1 = full JSON schemas for the **top-K** semantic router matches; Tier 2 = **allowed tool names only** (plus the deliberate `args: {}` schema-request path). It also flipped **empty semantic matches**: previously **conversational** (no tools in the system prompt); now **tool mode with roster-only** (Tier 1 empty, names still visible).

**Why this can cause the wrong `memory:query` shape:** With default `tool_schema_top_k` = **3**, only three router hits get full schemas. `memory:query` often ranks **4th–5th** (vault/agenda/web above it), so it may land in **Tier 2** with **no** schema in context — the model guesses and can reuse **`memory:stage` mental model** (`staged_id`, `tags[]`). **`gatekeeper.rs` was not modified** in that commit; validation is doing its job.

**Tomorrow (pick what fits):** Bump `tool_schema_top_k` in `fcp.toml` or code; ensure recall-style utterances promote **memory:query** (or all memory tools) into Tier 1; tighten JIT / copy so `memory:query` `{ query, filter_tag? }` is unmistakable vs tag-based tools; recovery policy — no Idle “I remembered…” after a schema fault on recall tools.

---
