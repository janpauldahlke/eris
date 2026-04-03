# Bootstrap and executive layer

## Entry point (`main.rs`)

1. Parse CLI (`executive::cli::Cli`).
2. Load `AppConfig` via Figment: defaults → `.fcp/config.toml` (relative to **cwd**) → `FCP_*` env.
3. Initialize tracing under the vault’s `.fcp/telemetry/logs/` (see `telemetry::logger`).
4. Run **preflight** (`telemetry::preflight`). For **Chat**, preflight is skipped (daemons are checked in chat startup path).
5. Install `SIGINT` → `CancellationToken` for cooperative shutdown.
6. Dispatch `executive::router::execute_command`.

Forbidden in crate: `unsafe`; `unwrap`/`expect` outside tests (see workspace rules).

## Configuration (`config.rs`)

`AppConfig` holds:

- **LLM:** `ollama_host`, `model_name`, `num_ctx`, `generation_timeout_secs`, `embed_model_name`.
- **Loop limits:** `max_tool_rounds`, `max_recovery_attempts`, condensation thresholds.
- **Semantic:** `qdrant_url`, computed `qdrant_collection` = `fcp_vault_{workspace}`, `require_semantic_brain`, retry knobs.
- **Context optimization:** `optimize_context*` flags for `build_llm_view` (slim tool schemas, snippet caps).
- **API profiles:** `apis` map for `util::ApiHttpClient` (weather, Wikipedia, etc.).
- **Vault watch:** `VaultWatchConfig` (`vault_watch` in TOML) for paths debounced via `notify`.

`config_source_dir` is set to `std::env::current_dir()` after extract—**this is the vault root**.

## `vault_layout.rs`

Canonical paths from a workspace root:

- `.fcp/config.toml`, `.fcp/seal`, `.fcp/tools/agenda.json`, `.fcp/tools/alarms.json`
- `.fcp/telemetry/logs/`
- `.fcp/ephemeral_{workspace}.bin` — bincode snapshot of ephemeral cache

## `workspace.rs`

`init_workspace(vault_root, workspace, model)` creates a **nested** workspace under `vault_root/workspace/` with `00_Core`…`99_USER_UPLOADED`, seal file, Identity. Used for **multi-workspace** vault layouts; **chat** uses cwd as vault root directly (see router).

## Executive router (`executive/router.rs`)

### `Commands::Chat`

High-level sequence:

1. **Channels:** `tui_tx/tui_rx`, `action_tx/action_rx` for user actions.
2. **Terminal:** `ui::terminal::setup_terminal`.
3. **Seal / ignition:** If `.fcp/seal` missing → `ignition::run_ignition_sequence`, then reload config.
4. **Identity:** `identity_md::sync_identity_user_line`, load `vault_identity::read_identity_markdown_strict` into a `watch::channel` for `ContextAssembler`.
5. **Optional** `notify` watcher: `util::fs_watch::spawn_vault_identity_watch` for Identity.md + configured paths.
6. **Peripherals:** `peripherals::ensure_peripherals_for_chat` — start Ollama/Qdrant if not reachable.
7. **Engine:** `Ollama::new` + `OllamaClient::with_token_metrics`.
8. **SemanticBrain:** `SemanticBrain::new_with_connect_retries` or `None` if `require_semantic_brain` is false and connection fails.
9. **Boot ingest:** `semantic.ingest_vault(&workspace_root)` if brain online.
10. **Gatekeeper:** register all tools (vault, agenda, web, system, clock, weather, wiki, memory—memory tools conditional on semantic).
11. **Descriptors:** `ToolDescriptorRegistry::load_embedded` + `assert_covers_registered_tools`.
12. **ToolRouter:** optional; if `new` fails, orchestrator runs with full tool roster always.
13. **Heartbeat:** `heartbeat::spawn_heartbeat_monitor` → idle `watch` trigger when idle timeout exceeded.
14. **Alarm scheduler:** `alarm_scheduler::spawn_alarm_scheduler` reads `.fcp/tools/alarms.json`.
15. **Missed agenda hint:** `missed_agenda::startup_overdue_agenda_hint` (async spawn).
16. **Ephemeral snapshot daemon:** `memory::ephemeral::spawn_snapshot_daemon`.
17. **Orchestrator::new** with `vault_root` = cwd, `workspace` **string empty** so `ContextAssembler` resolves `vault_root/00_Core` (not `vault_root/default/00_Core`).
18. **Spawn** orchestrator loop that drains `action_rx` (Submit, Cancel, SystemInject, AgendaAlarmPending).
19. **TUI:** `TuiApp::run` until exit; then cancel token, restore terminal, shutdown managed daemons.

### Other commands

- **`Run` / `Tool`:** stub or minimal (`Tool` only recognizes `memory:query` as OK in one branch); real work is **Chat**.

## Ignition (`executive/ignition.rs`)

Interactive `inquire` prompts (in `spawn_blocking`) when no seal: agent name, user name, model, scaffold dirs, write config. Used for first-time vault setup.

## Peripherals (`executive/peripherals.rs`)

TCP checks against `ollama_host` and `qdrant_url`; may spawn child processes from `DaemonCommand` in config or Docker for Qdrant in some paths. `PeripheralLifecycle` tracks what this process started so shutdown can kill only those.

## Identity helpers

- `identity_md.rs` — sync user line in Identity.md.
- `vault_identity.rs` — strict read of identity file for snapshot.
