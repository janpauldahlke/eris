# Adding a tool (contributor checklist)

1. **Implement `Tool`** in `src/tools/<area>/` (`name`, `description`, `parameters_schema`, `execute`). Use `serde` + `JsonSchema` for args; route errors through `FcpError` (no `unwrap`/`expect` outside `#[test]`).
2. **Register** the tool in `src/executive/chat_session.rs` (`gatekeeper.register(...)` during chat bootstrap), in the same order as related tools if dependencies matter (e.g. conditional blocks for `google.enabled` / shared `workspace_auth` for Gmail + Calendar, semantic brain, `web_fetch_deprecated`).
3. **Descriptors:** add a TOML block to `DESCRIPTOR_TOMLS` in `src/tools/specs.rs` (`tool_name`, `short_description`, `when_to_use`, `when_not_to_use`, `routing_hints`, examples). Startup runs `ToolDescriptorRegistry::assert_covers_registered_tools` — **missing descriptor for a registered name fails boot**.
4. **Gatekeeper:** extend `state_allows_tool` in `src/tools/gatekeeper.rs` for each `AgentState` that may call the tool; update `test_policy_covers_all_current_tools` in the same file when the tool is always registered.
5. **Arg aliases (optional):** if models often send wrong JSON keys, extend **`normalize_tool_args`** in `gatekeeper.rs` before schema validation.
6. **ToolRouter (embedding text):** prefer rich **`routing_hints`** in the descriptor TOML (step 3). If the tool truly has no hints in the registry, add a **`fallback_triggers`** arm for your `name()` in **`src/tools/routing_phrases.rs`** — `ToolRouter::enrich_for_routing` pulls from there automatically. Only touch **`src/orchestrator/tool_router.rs`** when adding **global** lexical rules (short-input guard, URL / “visit the page” style web intent), not per-tool paraphrases. See **Routing layers** below for offer policy / dialog pairing / affinity.
7. **Tests:** schema / happy path; any filesystem writes under `#[test]` must use **`tempfile`** (workspace rules). Add a small unit case in `src/orchestrator/routing/` when you introduce a new dialog-pairing or affinity rule.
8. **Web Tools console:** if the tool belongs to an optional family, add or extend a **`should_register_*`** predicate in [`src/tools/registration.rs`](../../src/tools/registration.rs) and list the tool name in the matching family in [`src/ui/web/tools_config_schema.rs`](../../src/ui/web/tools_config_schema.rs). Add editable config keys to [`family_field_keys`](../../src/ui/web/tools_config_schema.rs) and the merge whitelist in [`src/ui/web/tools_config_merge.rs`](../../src/ui/web/tools_config_merge.rs) when operators should change them from the UI.

For architecture context: [docs/updated_architecture/05_TOOLS_GATEKEEPER_DESCRIPTORS.md](updated_architecture/05_TOOLS_GATEKEEPER_DESCRIPTORS.md).

---

## Routing layers (where to put what)

Tool **offer** selection is no longer “cosine ≥ threshold → lock GBNF onto those names” alone. Flow:

```text
user text
  → ToolRouter (embeddings + lexical injects)     src/orchestrator/tool_router.rs
  → RoutingSignals                                 src/orchestrator/routing/signals.rs
  → decide() / dialog pairing / demotion / margin  src/orchestrator/routing/{policy,dialog}.rs
  → RoutingOffer / RoutingDecision                 src/orchestrator/routing/decision.rs
  → slim overlays (cap seats domains, verb complete,  src/orchestrator/routing/overlays.rs
     web:find / pairing / moltbook latch)
  → slim prompt + GBNF subset                      step.rs + llama_gbnf_subset.rs
```

Logs carry `rule_id` and `offer_kind` on pre-LLM routing events — grep those when debugging.

### Checklist by change type

| You are adding / changing… | Put it here |
|---|---|
| Paraphrases the model should **embed toward** this tool | `routing_hints` in `specs.rs` (preferred). Keep `routing_phrases.rs` fallbacks aligned if you touch them. |
| **Global** lexical force (URL → `web:fetch`, news phrases, Moltbook brand floor) | `tool_router.rs` only |
| Prefix affinity for near-tie **cluster unions** (e.g. `agenda`∪`clock`∪`calendar` = `time`) | `routing/clusters.rs` → `affinity_group` |
| Follow-up after a **successful** tool (“delete that email”, “cancel that meeting”) | `routing/signals.rs` (cues) + `routing/dialog.rs` (ordered rules). Agenda wins over mail/calendar/doc. |
| Weak lone-hit demotion / margin / unsure fallback knobs | Config: `tool_match_threshold`, `tool_single_hit_floor`, `tool_match_margin`, `tool_unsure_fallback`. Logic: `routing/policy.rs` |
| Slim map ↔ GBNF pairing + **domain verb completion** (`web:find` with fetch, `doc:read`→`vault:write`, seated `vault:*` / `memory:*` full verb sets, moltbook latch) | `routing/overlays.rs` (single source; `llama_gbnf_subset` re-exports). Cap seats domains; completion fills all state-allowed verbs for those domains (may exceed cap). |
| Decision type / log labels | `routing/decision.rs` |
| URL soft-compel / opinion-only / skip-fetch scorecard | `routing/policy.rs` (`should_soft_compel_web_fetch`); log `PRELLM_URL_SOFT_COMPEL` / `PRELLM_URL_SKIP_FETCH` from `step.rs`. Offline offer goldens: `src/benchmark/routing_offer_fixtures.rs` |

### Hint hygiene (do this when writing `routing_hints`)

- Prefer **domain-anchored** phrases: `"delete agenda item"`, `"delete that email"`, `"cancel meeting"`, `"search on Moltbook"`, `"search the web"`.
- Avoid bare bombs that collide across domains: lone `"remove"`, `"delete"`, `"reply"`, `"alarm"`, `"what is happening"`, `"find about"`.
- **Clock vs agenda:** wake-only / “no todo” for `clock:alarm`; “remind me to …” / errand framing for `agenda:remind_at`.
- **Doc delete:** keep `"document"` / `"pdf"` / `"unindex"` / `"ingested"` — never bare `"remove it"` (agenda dialog pairing owns that deictic).
- **Moltbook:** every hint should mention Moltbook/submolt so brand-less web search does not bleed.

### Dialog pairing rules of thumb

Pattern (see `routing/dialog.rs`):

1. Recent successful tools include `prefix:*` (session-scoped on the orchestrator).
2. User text matches **domain-anchored** continuation cues in `signals.rs`.
3. Offer that domain’s cluster (preferred tools first) — **not** the full time-affinity dump for calendar.
4. Doc pairing requires document vocabulary; bare “remove it” after `doc:list` must **not** steal from agenda.

Priority today: **agenda → mail → calendar → gated doc**.

### Config operators care about

| Knob | Default | Role |
|---|---|---|
| `tool_match_threshold` | `0.50` | Cosine floor for router hits |
| `tool_single_hit_floor` | `0.58` | Lone hit below this is demoted (avoids GBNF lock-in) |
| `tool_match_margin` | `0.05` | Near-tie window for affinity cluster union |
| `tool_unsure_fallback` | `full_roster` | After demotion: `full_roster` or `domain_cluster` |
| `tool_map_offer_cap` | (see config) | Cap on slim/GBNF offered names |

---

## Background events (alarms, future “hardware interrupts”)

Use the **single presentation outbound channel** (`mpsc::Sender<SessionEvent>` into the active view or multiplexer). Do **not** add a second semantic queue parallel to it for the same event type.

1. **Scheduler** (`src/orchestrator/alarms/scheduler.rs`): on due alarms, **`presentation_tx.try_send(SessionEvent::SystemAlarm(payload))`** only (`AlarmPayload::Plain` or `AgendaLinked`). If full, log and drop; **never** `await` send from the scheduler loop.
2. **Views** translate alarms into orchestrator input on the **`UserAction`** channel:
   - **Terminal:** `src/ui/terminal/app.rs` maps `SessionEvent::SystemAlarm` → `UserAction::SystemInject` / `AgendaAlarmPending` via `try_send` on `action_tx` (same cases as `alarm_payload_to_user_action`).
   - **Web + Discord:** when a multiplexer is active, `src/presentation/multiplex.rs` relays `SystemAlarm` to **`user_action_tx`** once and fans out `SessionEvent` copies as configured — follow that pattern so alarms are not duplicated.
3. **`chat_session`** owns the task that receives **`UserAction`** on its channel, applies `SYSTEM_ALARM_PREFIX` for injects (`src/presentation/mod.rs`), and calls `orchestrator.step`. This path does not use the idle **heartbeat** `watch` interrupt; it does not cancel an in-flight LLM generation unless your design adds that explicitly.

**Lexical note:** injected alarm lines are short; `ToolRouter::short_input_guard_conversational_only` tends to treat them as conversational-only for routing — usually desirable for a quick nudge instead of deep tool escalation. Moltbook-labeled alarms are an exception (tool mode enabled).

**Wiring reference:** `spawn_alarm_scheduler` is called from **`src/executive/chat_session.rs`** (not the executive router’s thin `Chat` branch). Pair with `clock:timer` / `clock:alarm` and `AlarmPayload` shapes in `src/presentation/mod.rs`.
