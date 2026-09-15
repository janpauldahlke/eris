# Agenda pairing steals the offer; OpenRouter Auto then grabs vault

Status: **OPEN — elephant (2026-09-15)**  
Log: `vaults/billy/.fcp/telemetry/logs/fcp_core.log.2026-09-15` turns 24–29 (~20:37–20:50Z)  
Config: Billy `llm_backend = "OpenRouter"`, `slim_tool_prompt = true`, `tool_map_offer_cap = 5`  
Audience: humans + AI agents working on Eris routing / OpenRouter tool hops

Do not conflate with [`TOOL_OFFER_CAP_DROPS_WRITES.md`](TOOL_OFFER_CAP_DROPS_WRITES.md) (domain verb amputation under cap — already overlay-fixed). This is a **policy rewrite** that drops a whole domain *before* overlays, plus an OpenRouter hop-chop that makes the model lie about the roster.

---

## Symptom

Operator asks about **skills** / a vault path. Model answers that `skills:list` / `skills:read` are not in the session and need a restart. Operator sees `src/skills` and the three gatekeeper tools and assumes they were never registered.

They **are** registered (`chat_session.rs` always registers `skills:list` / `skills:read` / `skills:create`; Chat/Idle allow them). The model is confusing **not offered this hop** with **not in the binary**.

---

## Elephant 1 — leftover agenda word steals the offer

`try_dialog_pairing` runs **first** in `decide()` (`src/orchestrator/routing/policy.rs`). `rule_agenda` fires when:

- recent successful agenda tool exists (`signals.recent_had_agenda`)
- user text contains a continuation cue (`has_agenda_continuation_intent`)

Cues are substring matches (`src/orchestrator/routing/signals.rs`): `done`, `complete`, `finished`, `remove`, `from the agenda`, …

A long message *about* the agenda-self-loop skill still contains those words. Router cosine already ranked `skills:list` (turn 24: 0.608). Pairing **rewrites** the offer to the agenda cluster only:

```
rule_id = AGENDA_DIALOG_PAIRING
raw_matched includes skills:list / skills:read / skills:create
offered = [agenda:remove, agenda:complete, agenda:list, agenda:push, agenda:remind_at, agenda:remind_self]
```

Skills (and vault) never reach slim assembly / native `tools`. The model then invents a frozen “this session’s tool set is clock+agenda+vault” story and repeats it on later hops even when skills *are* offered (turns 25, 27, 28).

### Likely fix directions (do not implement in this note)

- Pairing should **union** agenda with other strong cosine hits, not replace them.
- Or: skip pairing when a non-agenda domain scores above a floor (`skills:list` 0.60+).
- Or: tighten cues (bare `done` / `complete` in a 600-char design note is not “close that task”).
- Or: pairing expires after N non-agenda user turns.

---

## Elephant 2 — OpenRouter `tool_choice=Auto` + post-tool talk hop

On OpenRouter Chat, after a **successful** tool batch:

```
should_force_post_tool_talk = is_openrouter() && state == Chat
→ PostToolTalkPass  (tools stripped, model must answer)
```

(`src/orchestrator/core/tool_dispatch.rs`)

One user prompt = **one** tool batch. `vault:list` cannot be followed by `vault:read` in the same prompt. Next user message starts a new round; `tool_choice=Auto` makes Qwen grab whatever vault/skills the slim offer still has.

Tonight’s ping-pong:

| turn | what happened |
| --- | --- |
| 27 | `skills:list` top-ranked (0.775); model called `vault:list` on `10_Topology/skills`; talk-pass stripped tools; asked permission to read |
| 28 | `skills:list` 0.815 offered; model claimed tools not in session |
| 29 | two `vault:read`s; talk-pass again |

This is why every follow-up looks like another `[tool] vault:list / vault:read`. Not a missing registration. The hop-chop plus Auto.

### Likely fix directions

- Do not force talk-pass when the successful batch was a *lookup* (`vault:list` / `skills:list` / `vault:search`) and the user still needs a read.
- Or: allow one chained follow-up hop for the same domain (list → read).
- Or: `tool_choice` not Auto when the user message is meta (“do you have X?”) vs operational.

---

## Side snag (same evening, not the elephant)

Turn 29: user pasted a long analysis; embed server 500:

`input (604 tokens) is too large to process. increase the physical batch size (current batch size: 512)`

→ `PRELLM_MATCH_ERROR` / full roster. Separate from pairing; still makes routing lie.

---

## Ground truth from that log

- Skills tools **were** in `raw_matched` on the skills questions.
- Billy **did** call `skills:list` successfully earlier the same day (afternoon, ~13:26Z).
- Runtime files live at `vaults/billy/10_Topology/skills/` (seeded at ignition). `src/skills/` is Rust, not the agent-facing tool.

When you come back: start at `rule_agenda` + `has_agenda_continuation_intent`, then `should_force_post_tool_talk`. Reproduce with Billy OpenRouter, an agenda turn, then “list the skills in 10_Topology/skills” without saying `done`/`complete`.
