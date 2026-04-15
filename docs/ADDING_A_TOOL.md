# Adding a tool (contributor checklist)

1. **Implement `Tool`** in `src/tools/<area>/` (`name`, `description`, `parameters_schema`, `execute`). Use `serde` + `JsonSchema` for args; route errors through `FcpError` (no `unwrap`/`expect` outside `#[test]`).
2. **Register** the tool in `src/executive/chat_session.rs` (`gatekeeper.register(...)` during chat bootstrap), in the same order as related tools if dependencies matter (e.g. conditional blocks for `google.enabled`, semantic brain, `web_fetch_deprecated`).
3. **Descriptors:** add a TOML block to `DESCRIPTOR_TOMLS` in `src/tools/specs.rs` (`tool_name`, `short_description`, `when_to_use`, `when_not_to_use`, `routing_hints`, examples). Startup runs `ToolDescriptorRegistry::assert_covers_registered_tools` — **missing descriptor for a registered name fails boot**.
4. **Gatekeeper:** extend `state_allows_tool` in `src/tools/gatekeeper.rs` for each `AgentState` that may call the tool; update `test_policy_covers_all_current_tools` in the same file.
5. **ToolRouter (embedding text):** prefer rich **`routing_hints`** in the descriptor TOML (step 3). If the tool truly has no hints in the registry, add a **`fallback_triggers`** arm for your `name()` in **`src/tools/routing_phrases.rs`** — `ToolRouter::enrich_for_routing` pulls from there automatically. Only touch **`src/orchestrator/tool_router.rs`** when adding **global** lexical rules (short-input guard, URL / “visit the page” style web intent), not per-tool paraphrases.
6. **Tests:** schema / happy path; any filesystem writes under `#[test]` must use **`tempfile`** (workspace rules).

For architecture context: [docs/updated_architecture/05_TOOLS_GATEKEEPER_DESCRIPTORS.md](updated_architecture/05_TOOLS_GATEKEEPER_DESCRIPTORS.md).

## Background events (alarms, future “hardware interrupts”)

Use the **single presentation outbound channel** (`mpsc::Sender<SessionEvent>` into the active view or multiplexer). Do **not** add a second semantic queue parallel to it for the same event type.

1. **Scheduler** (`src/orchestrator/alarms/scheduler.rs`): on due alarms, **`presentation_tx.try_send(SessionEvent::SystemAlarm(payload))`** only (`AlarmPayload::Plain` or `AgendaLinked`). If full, log and drop; **never** `await` send from the scheduler loop.
2. **Views** translate alarms into orchestrator input on the **`UserAction`** channel:
   - **Terminal:** `src/ui/terminal/app.rs` maps `SessionEvent::SystemAlarm` → `UserAction::SystemInject` / `AgendaAlarmPending` via `try_send` on `action_tx` (same cases as `alarm_payload_to_user_action`).
   - **Web + Discord:** when a multiplexer is active, `src/presentation/multiplex.rs` relays `SystemAlarm` to **`user_action_tx`** once and fans out `SessionEvent` copies as configured — follow that pattern so alarms are not duplicated.
3. **`chat_session`** owns the task that receives **`UserAction`** on its channel, applies `SYSTEM_ALARM_PREFIX` for injects (`src/presentation/mod.rs`), and calls `orchestrator.step`. This path does not use the idle **heartbeat** `watch` interrupt; it does not cancel an in-flight LLM generation unless your design adds that explicitly.

**Lexical note:** injected alarm lines are short; `ToolRouter::short_input_guard_conversational_only` tends to treat them as conversational-only for routing — usually desirable for a quick nudge instead of deep tool escalation.

**Wiring reference:** `spawn_alarm_scheduler` is called from **`src/executive/chat_session.rs`** (not the executive router’s thin `Chat` branch). Pair with `clock:timer` / `clock:alarm` and `AlarmPayload` shapes in `src/presentation/mod.rs`.
