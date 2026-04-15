# Self-review and documentation quality

This section records a **second pass** over the doc set for consistency, known gaps, and intentional limits.

## Consistency checks

- **Vault root:** Documented as `cwd` at `AppConfig::load` everywhere; matches `active_vault()` and chat router behavior.
- **ContextAssembler path:** With `workspace` `""` in chat, `core_dir` = `vault_root / 00_Invariants` — verified against `ContextAssembler::new` (`orchestrator/context/assembler.rs`).
- **ToolRouter input:** Pre-LLM routing calls `match_tools` with **user** text, not model `thought`; the API name `thought` in `match_tools` is slightly misleading—callers pass user input for the first pass.
- **Binary name vs clap:** `Cargo.toml` package and `#[command(name = "eris")]` match.

## Possible drift / incomplete product areas

- **`Commands::Run`** and parts of **`Commands::Tool`** are stubs; docs state this—operators should not expect full CLI parity with chat.
- **`engine::router::ReasoningRouter`:** Confirmed **not** used outside `engine/router.rs` tests; `enable_reasoning_fsm` in config does not currently attach this FSM to chat streaming.
- **Embedding dimensions:** Semantic brain uses 768-dim vectors; changing `embed_model_name` to a different dimension may require collection recreation—operational note, not enforced in code excerpt reviewed.
- **Discord / web / TUI parity:** All surfaces share `UserAction` / `SessionEvent`, but only some events map 1:1 to Discord (e.g. assistant lines via mux); operator docs should not imply full rich-client parity.
- **Setup welder vs ignition:** Welder runs only when seal is missing **and** interactive preconditions hold; ignition still creates the seal inside `start_chat_session`.

## Diagram limitations

- Mermaid state diagram for `AgentState` is **illustrative**; real transitions depend on `orchestrator/core/` and `orchestrator/loop/*` branches.
- Layer diagram omits `ingest` and `util` for clarity—they sit between tools/memory and filesystem/network.

## Improvements made during review

- Added explicit note on ToolRouter parameter naming vs user input.
- Called out `ephemeral_{workspace}.bin` path for operators debugging session restore.
- Clarified preflight skip for Chat vs peripheral checks in router.
- Corrected vault layout naming (`00_Invariants`, v2 ingest roots) and Qdrant collection (`qdrant_collection_v2` / `fcp_vault_v2_*`).
- Documented ephemeral **promotion/decay** vs **snapshot** ticks and **suppression during `Orchestrator::step`** (`Arc<AtomicBool>`).
- Documented **tool-round UI split**: `message_to_user` on deck, `Tools: …` on status, duplicate deck suppression.
- Documented **`presentation/`** module, **`eris chat --web`**, optional **Discord sidecar**, **`routing_phrases`**, **`idle_heartbeat_enabled`**, and **setup welder** vs ignition ordering.

For a **critical engineering** take (debt, refactors, redesign triggers), see [09_CRITICAL_REVIEW.md](./09_CRITICAL_REVIEW.md).

## When to update this folder

Update when:

- New tools or descriptor requirements change.
- Orchestrator pre-LLM routing or gatekeeper state matrix changes.
- Vault path semantics change (would be a major version concern).
- Presentation contracts (`SessionEvent` / `UserAction`), web routes, or Discord wiring change.
