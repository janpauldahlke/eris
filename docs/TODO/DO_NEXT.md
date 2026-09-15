# DO_NEXT — what to actually change (2026-09-16)

Status: **OPEN — ranked work, not a design essay**  
Origin: Billy’s “improve Eris” list (Q8 / OpenRouter, night of 2026-09-15) vs a peer cut of the same session.  
Detail on the two elephants: [`AGENDA_PAIRING_STEALS_OFFER.md`](AGENDA_PAIRING_STEALS_OFFER.md)  
Log: `vaults/billy/.fcp/telemetry/logs/fcp_core.log.2026-09-15` turns 24–29

Billy’s #1 is the right *wall*. His proposed knobs are mostly wrong. Do 1–2 and that night’s session doesn’t happen.

---

## Do these, in this order

1. **`AGENDA_DIALOG_PAIRING` must not replace the offer.**  
   Union with other strong cosine hits, or skip pairing when a non-agenda domain is already ≥ ~0.58. Cheap variant: stop treating bare `done` / `complete` in a long design note as “close that task.”  
   Code: `src/orchestrator/routing/dialog.rs` `rule_agenda`, `signals.rs` `has_agenda_continuation_intent`.  
   Billy did not name this. It is why he swore `skills:list` was not registered.

2. **OpenRouter Chat: no talk-pass after a lookup.**  
   After a successful `vault:list` / `skills:list` / `vault:search` / `skills:read` / `vault:read`, allow another tool hop in the **same** user turn (list → read). Keep the hard stop for writes, mail, agenda mutations.  
   Code: `should_force_post_tool_talk` in `src/orchestrator/core/tool_dispatch.rs`.  
   Billy asked for “budget N hops.” Billy already has `max_tool_rounds = 40`. Reflect already skips the cap. The wall is Chat + OpenRouter forcing answer-now after hop **1**.

3. **Offer honesty.**  
   Meta questions (“what tools / do you have X”) should lexically force `skills:list` the way web/news already get forced. JIT-inject `tool-catalog-orientation` on those turns. Slim subset is a hop roster, not a session roster.

4. **Don’t prefetch truncated skill bodies as `[RELEVANT_LEARNED_MEMORY]`.**  
   Full skill, or a pointer (`skills:read agenda-self-loop`), not a three-line cut at “2. Writ…”. That fragment trained him that the skill system is one-way.

5. **Embed batch vs long pastes.**  
   604 tokens into physical batch 512 → `PRELLM_MATCH_ERROR` → routing lies. Chunk or refuse. Don’t silently full-roster.

6. **Restart pointer.**  
   One durable “current thread” (task_id + intent + next action), injected at ignition. Reuse `agenda:remind_self`. Don’t invent a new memory model for this.

---

## Do not do yet

- **Lethe / kill session→scratch→promote.** Design thread (Rune). Staging is load-bearing; `vault:write` is already the bypass. Billy documents the ladder in `vaults/billy/30_Synthesis/rules_of_the_house.md` and uses it.
- **“Lean mode for Q8.”** Slim offer *is* the lean mode. It is why Q8 treated a hop subset as a frozen toolset. Honesty first, density later.
- **Voice-calibration harness hook.** Vault note + optional system inject. Not Rust.
- **Spawn-researcher / sub-agent.** Correct, last. One model must be able to list→read in a single Chat turn first.

---

## Billy’s seven, scored

| # | His item | Verdict |
| --- | --- | --- |
| 1 | Multi-step tool chains | Right wall, wrong knob (`max_tool_rounds` already 40) |
| 2 | Lethe over tiering | Philosophy. Not next. |
| 3 | On-demand retrieval vs sidebar | Half: truncation is harmful; killing prefetch is not |
| 4 | Thread as primitive | Real; smaller than he thinks; reuse agenda |
| 5 | Feedback into voice | Vault, not harness |
| 6 | Lean mode for strong models | Trap until the offer is honest |
| 7 | Sub-agent | Last |

Start at (1) pairing, then (2) talk-pass. Reproduce: Billy OpenRouter, an agenda turn, then “list skills in `10_Topology/skills`” without saying `done`/`complete`.
