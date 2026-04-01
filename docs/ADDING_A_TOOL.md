# Adding a tool (contributor checklist)

1. **Implement `Tool`** in `src/tools/<area>/` (`name`, `description`, `parameters_schema`, `execute`). Use `serde` + `JsonSchema` for args; route errors through `FcpError`.
2. **Register** the tool in `src/executive/router.rs` (`gatekeeper.register`).
3. **Descriptors:** add a TOML block to `DESCRIPTOR_TOMLS` in `src/tools/specs.rs` (`tool_name`, `short_description`, `when_to_use`, `routing_hints`, examples). Boot fails if registered tools are not covered.
4. **Gatekeeper:** extend `state_allows_tool` in `src/tools/gatekeeper.rs` for each `AgentState` that may call the tool; update `test_policy_covers_all_current_tools`.
5. **Tool router:** add fallback hints in `ToolRouter::enrich_for_routing` in `src/orchestrator/tool_router.rs` when useful (descriptors already supply hints for embeddings).
6. **Tests:** schema/happy path; any filesystem writes under `#[test]` must use `tempfile`.

## Background events (alarms, future “hardware interrupts”)

Use the **TUI relay**—do not add a second queue inside the orchestrator for the same semantic event.

1. Background task sends **`tui_tx.try_send(TuiEvent::SystemAlarm(label))`**. If full, log `tracing::error!(...)` and continue; **never block** the scheduler on the TUI.
2. TUI handles `SystemAlarm` with **`action_tx.try_send(UserAction::SystemInject(label))`** only (dumb pipe).
3. The orchestrator task consumes **`UserAction::SystemInject`** on `action_rx`, formats with `SYSTEM_ALARM_PREFIX` in `src/ui/events.rs`, and runs `step`. **No** `watch` / interrupt path for alarms; **no** abort of in-flight LLM generation.

**Lexical bypass:** the prefixed alarm line is short; `SHORT_INPUT_GUARD` in `tool_router.rs` tends to force conversational mode—desirable for a quick user-facing nudge instead of deep tool escalation.

**Reference implementation:** `clock:timer`, `clock:alarm`, and `src/orchestrator/alarm_scheduler.rs`.
