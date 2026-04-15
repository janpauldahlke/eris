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
- **Semantic:** `qdrant_url`, computed `qdrant_collection_v2` = `fcp_vault_v2_{workspace}` (see `AppConfig` merge), `require_semantic_brain`, retry knobs.
- **Ephemeral promotion:** TTLs per `EphemeralTier`, score thresholds, `promotion_eval_interval_secs`, decay per tick, optional `turn_end_mention_enabled` and `staged_memory_prompt_max_chars` (see `config.rs` and [04_MEMORY_SUBSYSTEM.md](./04_MEMORY_SUBSYSTEM.md)).
- **Context optimization:** `optimize_context*` flags for `build_llm_view` (slim tool schemas, snippet caps).
- **API profiles:** `apis` map for `util::ApiHttpClient` (weather, Wikipedia, etc.).
- **Vault watch:** `VaultWatchConfig` (`vault_watch` in TOML) for paths debounced via `notify`.
- **Web UI:** `web_bind_addr`, `web_port`, `web_open_browser` (Figment env keys follow the crate’s `FCP_` merge for these fields where implemented).
- **Discord:** nested `discord` struct (`enabled`, `application_id`, `public_key`, `channel_id`, `channel_name`, `bot_token`, `outbound_queue_capacity`).
- **Google Workspace (Gmail tools):** `google` (`enabled`, `service_account_key`, `impersonate_user`).
- **Idle heartbeat:** `idle_heartbeat_enabled` (default **false**), `idle_timeout_secs` — controls whether `spawn_heartbeat_monitor` runs.

`config_source_dir` is set to `std::env::current_dir()` after extract—**this is the vault root**.

## `vault_layout.rs`

Canonical paths from a workspace root:

- `.fcp/config.toml`, `.fcp/seal`, `.fcp/tools/agenda.json`, `.fcp/tools/alarms.json`
- `.fcp/telemetry/logs/`
- `.fcp/ephemeral_{workspace}.bin` — bincode snapshot of ephemeral cache

## `workspace.rs`

`init_workspace(vault_root, workspace, model)` creates a **nested** workspace under `vault_root/workspace/` with `00_Invariants` (contains `Identity.md`), `10_Topology`, `20_Discourse`, `30_Synthesis`, `.fcp/` seal and tools dir. Used for **multi-workspace** vault layouts; **chat** uses cwd as vault root directly (see router).

## Executive router (`executive/router.rs`)

### `Commands::Chat`

1. **Vault root:** `config.active_vault()` (chat launch **cwd**); log workspace id.
2. **Seal missing:** `setup_welder::run_welder_before_chat` — optional interactive first-run flow (**skipped** when `ERIS_SKIP_SETUP=1`, `CI=true`, or stdin is not a TTY). It gathers `WelderReport` (Ollama/Qdrant reachability, `ollama`/`qdrant`/`docker` on `PATH`) and runs `inquire` prompts in `spawn_blocking` to confirm the vault directory and workspace hint. It does **not** create the seal by itself.
3. **`ChatViewMode`:** from CLI — `Terminal` (default) vs `Web` (`eris chat --web`).
4. **Presentation channel:** `mpsc::channel` `(presentation_tx, presentation_rx)` capacity 100 — core sends `SessionEvent`; views send `UserAction` on a separate channel created inside `start_chat_session`.
5. **Discord config:** `AppConfig::validate_discord_sidecar` (strict when `discord.enabled` but incomplete). If enabled but bot token missing, log `fcp.discord.sidecar_skipped` and run without gateway.
6. **Discord mux handles (optional):** when `discord_sidecar_should_run()`, allocate outbound + typing channels for `run_discord_sidecar`.
7. **Branch on view:**
   - **Web:** `start_chat_session` → `ui::web::run_web_chat` or, with Discord, `spawn_presentation_multiplex` + `run_web_chat_with_broadcast` + `run_discord_sidecar` (broadcast `SessionEvent` to SSE subscribers; `IncomingMessage` also `try_send` to Discord outbound).
   - **Terminal:** `setup_terminal` → `start_chat_session` → either `TuiApp::run(presentation_rx, …)` alone, or with Discord: multiplex fans `presentation_rx` to TUI + Discord (alarms relayed once via mux to `user_action_tx`; TUI may omit duplicate `SystemAlarm` forward when mux handles it).
8. **Inside `start_chat_session`** (shared by both views — see `executive/chat_session.rs`):
   - If `.fcp/seal` still missing: `ignition::run_ignition_sequence`, then **reload** `AppConfig`.
   - Identity sync + `watch` snapshot; optional vault `notify` watcher.
   - Peripherals, engine, semantic brain, boot ingest, **gatekeeper** registration (vault, agenda, web, system, clock, weather, wiki, DB, mail when Google enabled, memory when semantic online), descriptors, ToolRouter.
   - **Idle heartbeat:** `spawn_heartbeat_monitor` **only if** `idle_heartbeat_enabled` is `true` (default in `AppConfig` is **false**); otherwise log that idle injection is off while Esc cancel remains.
   - Alarm scheduler, missed-agenda hint, ephemeral snapshot daemon + `promotion_suppressed_during_step` shared flag.
   - **`Orchestrator::new`** with `vault_root` = cwd, orchestrator `workspace` argument **`""`** so `ContextAssembler` uses `vault_root/00_Invariants` (flat v2 layout for normal chat).
   - Spawn orchestrator loop consuming `UserAction` from the view(s).
9. **Shutdown:** cancel token, `restore_terminal` on TUI path, `PeripheralLifecycle::shutdown_started_peripherals`, join mux / Discord tasks as wired in the branch.

### Other commands

- **`Run` / `Tool`:** stub or minimal (`Tool` only recognizes `memory:query` as OK in one branch); real work is **Chat**.

## Ignition (`executive/ignition.rs`)

Interactive `inquire` prompts (in `spawn_blocking`) when no seal: agent name, user name, model, scaffold **v2** dirs (`00_Invariants`, `10_Topology`, …), write config + seal. Runs inside `start_chat_session` after optional welder.

## Setup welder (`executive/setup_welder/`)

**When:** router calls it only if `.fcp/seal` is absent **before** `start_chat_session`.

**Purpose:** environment report + human confirmation of vault root / workspace hint; steers operators away from running the binary from Downloads-only paths.

**Skips:** `ERIS_SKIP_SETUP=1`, `CI=true`, non-interactive stdin — returns `IgnitionWorkspaceHint::from_cli` immediately.

## Peripherals (`executive/peripherals.rs`)

TCP checks against `ollama_host` and `qdrant_url`; may spawn child processes from `DaemonCommand` in config or Docker for Qdrant in some paths. `PeripheralLifecycle` tracks what this process started so shutdown can kill only those.

## Identity helpers

- `identity_md.rs` — sync user line in Identity.md.
- `vault_identity.rs` — strict read of identity file for snapshot.
