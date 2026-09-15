# Tool offer cap drops write tools (persist / memorize)

Status: **ADDRESSED in overlays (2026-08-26)** — domain verb completion for seated domains  
Prior analysis: 2026-08-24 (do not conflate with llama.cpp long-context merge)  
Log (original): `vaults/unknown/.fcp/telemetry/logs/fcp_core.log.2026-08-24` (after `--testrun2---`, turns ~6–9)  
Log (reconfirm): `vaults/unknown/.fcp/telemetry/logs/fcp_core.log.2026-08-26` (curate `memory_exploration.md`, turn 15)  
Config under test then: `tool_map_offer_cap = 5` / later `8`, slim phrase map on

Audience: humans + AI agents working on Eris/fcp routing

---

## Symptom

User asks to **recon a site and persist** (or later: “memorize / persist findings in the vault”,
“curate this note”).
Model researches with `web:fetch` / `web:find` or reads with `vault:search`, then either
reports it **cannot** call `memory:stage` / `vault:write` because they are “not in the
toolset”, or plans `vault:write` in thought while GBNF only lists read/search verbs.
Operator sees writes in router `offered` / `raw_matched` lists in logs and assumes a
Reflect / gatekeeper bug — it is not.

## Clear picture (pipeline)

```
user text
  → ToolRouter::match_tools
  → apply_routing_policy (RANKED_SUBSET | AFFINITY_MARGIN_UNION | …)
  → RoutingDecision.matched_tool_names()     ← often INCLUDES one vault/memory verb
  → apply_offer_overlays
        · seed = highest-ranked prefix under cap
        · complete all state-allowed verbs for each seed domain  ← fix lives here
        · pairing overlays (web:find, doc:read→vault:write, …)
  → assemble_slim_tool_map / GBNF subset     ← model only sees this list
```

Routing runs **once** per user `step()`; mid-turn tool hops **reuse** the same pre-LLM offer
(`src/orchestrator/core/step.rs`). Sticky web-only offers after a fetch remain a second
failure mode when **no** vault/memory seed ever entered the capped prefix.

### Confirmed log cases (pre-fix)

**Turn 8 — “memorize…” (`AFFINITY_MARGIN_UNION`)**

- Semantic hits included `vault:write(0.641)`, `memory:stage(0.562)`.
- Policy union offered 10 vault+memory tools **including** writes.
- After `rerank_offered_by_cosine`, order was roughly:
  `vault:search, vault:read, vault:list, memory:query, memory:staged_list, …, vault:write, …, memory:stage`
- Slim view (`cap=5`): **only reads** —
  `vault:search, vault:read, vault:list, memory:query, memory:staged_list`
- Model called `staged_list` + `query`, then correctly said it cannot write.

**Turn 9 — “pls persist … in the vault” (`RANKED_SUBSET`)**

- Hits: `web:fetch(0.670)` … `vault:write(0.560)`, `memory:stage(0.535)`.
- Slim top-5: `web:fetch, vault:search, web:find, vault:read, web:search` — **still no writes**.
- Model kept reading vault paths; Recover widened schemas (including `vault:write` /
  `memory:stage`) but then **empty-action** Recover loops → recovery budget exhausted.

**2026-08-26 turn 15 — “curate … memory_exploration.md” (`RANKED_SUBSET`, cap=8)**

- `raw_matched` never included `vault:write` (embed miss), but **did** include `vault:search`
  and several `memory:*` verbs.
- Blind prefix kept `vault:search` / `memory:stage` and dropped the rest of the vault verb
  set — classic domain amputation, not “write never matched then truncated.”
- Model thought “I will vault:write”, called `vault:search`, then empty-action Recover.

**Earlier recon turn** — affinity / ranked web cluster sticky for whole hop chain; writes never
entered the slim view even when thought said “I will memory:stage”.

## Root causes (ranked)

1. **Blind prefix take after affinity expand** (`apply_offer_overlays`): `.take(tool_map_offer_cap)`
   on cosine-ordered names amputated sibling verbs of a domain that already had a seat.
2. **Unscored sibling inheritance** (`rerank_offered_by_cosine` in
   `src/orchestrator/routing/policy.rs`): cluster siblings that never hit embed get
   `domain_best − ε`, so e.g. `memory:staged_list` can outrank a real `vault:write` hit.
3. **Embedding bias**: “vault / memorize / persist / website” still cosine-closer to
   search/read/query than stage/write (no write-intent overlay analogous to `doc:read`→`vault:write`
   or `web:fetch`→`web:find`).
4. **Sticky mid-turn offer**: persist-after-fetch never re-routes; web top-N stays locked.
5. **Empty-action paralysis** (separate): even with write in the Recover/target set, the model
   emits `Task` + thought + `tool_calls:[]` until recovery budget exhausts. Not fixed by offer
   overlays.

Not causes: Reflect allowlist (gatekeeper stayed `Chat`); missing write tools in registry;
GBNF “disabling” writes.

## Fix shipped (2026-08-26)

**Domain verb completion** in [`src/orchestrator/routing/overlays.rs`](../../src/orchestrator/routing/overlays.rs):

1. Cap still selects the **highest-ranked seed prefix** (which *domains* get a seat).
2. For each domain in that prefix (rank order), offer **all** state-allowed `domain:*`
   registered verbs: seed tools first, then remaining siblings (`cluster_members`).
3. Completed domain sets may **exceed** `tool_map_offer_cap` — cap no longer amputates verbs
   of a seated domain.
4. Existing pairing overlays (`web:find`, `doc:read`→`vault:write`, `vision:see`→`media:catalog`,
   Moltbook latch) still run after completion.

Telemetry: `routing.offer.domain_verb_complete` when completion grows the offer.

Tests: `vault_seed_completes_all_vault_verbs_past_cap` and siblings in `overlays.rs`.

### Still open

| Gap | Notes |
|-----|--------|
| Embed never seats vault/memory at all | Domain completion cannot invent a domain with zero seed hits (turn 16 “try again” → skills/web). |
| Sibling inherit demotion | Optional follow-up in `rerank_offered_by_cosine`. |
| Mid-turn re-offer | Persist-after-fetch sticky web offer. |
| Empty-action Recover loops | Protocol / generation issue once tools are visible. |

## Why raising `tool_map_offer_cap` alone was uncertain

| Cap | Likely effect |
|-----|----------------|
| 5 → 8 | Would have kept `vault:write` on turn 8 (was ~#7). Helps that case. |
| 5 → 10+ | Still fails when web+vault reads fill the prefix (turn 9 pattern), or when sticky web affinity omits memory/vault entirely. |
| 0 (uncapped) | Avoids truncate but blows slim prompt / GBNF size — defeats the improved router’s point. |

Domain verb completion keeps the cap as a **domain seat** budget, not a **verb amputation** budget.

## Related

- Soak note in `docs/TODO/REFACTOR_LLAMACPP_CONTEXT_HANDLING.md` (A3 / wiki dropped by cap=5) — same class.
- `docs/HOW_TO/ADDING_A_TOOL.md` — routing layers + `tool_map_offer_cap`.
- `docs/TODO/HANDOVER-doc-summarize-v1.md` — existing `doc:read`→`vault:write` auto-offer precedent.
