# Tools, gatekeeper, descriptors

## `Tool` trait (`tools/traits.rs`)

Each tool implements:

- `name()` — static str, e.g. `vault:read`
- `description()` — static str for OpenAI-style `description`
- `parameters_schema()` — `schemars::RootSchema` → JSON Schema for args
- `execute(Value) -> Result<String>`
- Optional `context_view_hint()` for LLM view trimming (`ToolContextViewHint`)

## Gatekeeper (`tools/gatekeeper.rs`)

- **Registry:** `HashMap<String, Arc<dyn Tool>>`.
- **`get_allowed_tools(state)`** — filters tools by **`AgentState`**:
  - **Chat:** all registered tools **except** `agenda:complete` (prevents completing before user turn semantics). Includes mail/DB/etc. when registered.
- **Reflect:** sandbox set: memory, `vault:read` / `vault:list`, `web:artifact_query`, agenda mutations, clocks, weather, wiki, `system:health`, `db:find_connections`, **`mail:check` / `mail:read` / `mail:digest`**, **`calendar:list` / `calendar:get`**, **`vision:see` / `vision:display` / `media:catalog` / `media:meta`** — **no** `vault:write`, **`web:fetch`**, **`news:today`**, **`mail:write` / `mail:delete` / `mail:move`**, **`calendar:create` / `calendar:update` / `calendar:delete`**.
- **Idle:** Reflect set **plus** `vault:write`, `web:fetch`, `news:today`, `agenda:complete`, **`mail:write` / `mail:delete` / `mail:move`**, **`calendar:create` / `calendar:update` / `calendar:delete`**.
  - **Recover:** all registered (recovery pass).
- **`execute_tool`:** state check → **`normalize_tool_args`** (coerce common LLM mistakes) → JSON Schema validate (`jsonschema`) → `tool.execute`.
- **Recover empty result** → `ToolFault` semantic guard.

### Arg normalization (`normalize_tool_args`)

Before schema validation, the gatekeeper coerces frequent model mistakes:

- **`web:search`:** `q` / `search_query` → `query`; strip unknown keys
- **`web:find`:** `url` / `artifact` → `artifact_id` when UUID-shaped; default empty `query`
- **`news:today`:** empty optional strings removed; `top_n` → `max_headlines`
- **Vision / media:** `path` / `file_path` → **`relative_path`** for `vision:see`, `vision:display`, `media:catalog`, `media:meta`

## Validation (`tools/validation.rs`)

Shared helpers (e.g. path mutability) used by vault tools.

## Tool families (by module)

| Module | Tools (examples) |
|--------|------------------|
| `tools/vault/` | `vault:read`, `vault:write`, `vault:list` |
| `tools/memory/` | `memory:stage`, `memory:staged_list`, `memory:commit`, `memory:commit_all`, `memory:query` |
| `tools/agenda/` | `agenda:push`, `agenda:list`, `agenda:remind_at`, `agenda:complete`, `agenda:remove` |
| `tools/web/` | `web:fetch`, `web:artifact_query` |
| `tools/news/` | `news:today` (homepage headlines + optional deep article fetches; shared fetch pipeline) |
| `tools/system/` | `system:health` |
| `tools/clock/` | `clock:now`, `clock:timer`, `clock:alarm` |
| `tools/weather/` | `weather:current`, `weather:forecast` (Open-Meteo via `ApiHttpClient`) |
| `tools/wiki/` | `wiki:summary` (Wikipedia REST) |
| `tools/db_rest/` | `db:find_connections` (Deutsche Bahn–style journey search via configured REST profile) |
| `tools/mail/` | `mail:check`, `mail:read`, `mail:write`, `mail:digest`, `mail:delete`, `mail:move` (Gmail via Google Workspace client; tools register only when `google.enabled` and credentials resolve) |
| `tools/calendar/` | `calendar:list`, `calendar:get`, `calendar:create`, `calendar:update`, `calendar:delete` (Google Calendar API; same `[google]` registration gate as mail) |
| `tools/vision/` | `vision:see` — multimodal describe via llama.cpp chat server (`file://` + `--mmproj`); `vision:display` — validated upload path + preview URL for web UI. Register only when `[vision] enabled` and `llm_backend = LlamaCpp` |
| `tools/media/` | `media:catalog` — create/update `40_MEDIA/{hash}/media.json` (v1: images); `media:meta` — patch existing cards. **Always registered**; Qdrant ingest of `40_MEDIA` is vision-gated |

Agenda and alarms persist JSON under `.fcp/tools/` (see `vault_layout`).

**Vision gate:** `VisionConfig::enabled` controls mmproj spawn (`executive/peripherals.rs`), web upload routes (`ui/web/vision_handlers.rs`), Discord attachment ingest (`ui/discord/attachment.rs`), **`40_MEDIA`** Qdrant ingest (`memory/semantic.rs`, `memory/reindex_watch.rs`), vision tool registration (`chat_session.rs`), and ingress rejection when disabled. Normalized JPEGs land under `[vision].upload_dir` (default `99_USER_UPLOADED/images/`). Operator doc: [docs/HOW_TO/VISION.md](../../HOW_TO/VISION.md).

**Remember nudge:** after successful `vision:see`, when the user turn matches remember intent and `media:catalog` is not already queued or done, `orchestrator/core/tool_dispatch.rs` injects `[FCP MEDIA — CATALOG NEXT]` from `llm_support/post_tool_guidance.rs`.

## Descriptors (`tools/descriptors.rs`, `tools/specs.rs`)

- **Embedded TOML** slices compiled into the binary (`DESCRIPTOR_TOMLS`).
- Per tool: `when_to_use`, `when_not_to_use`, `routing_hints`, good/bad examples.
- **`ToolDescriptorRegistry::load_embedded`** at startup; **must cover** every registered tool name or chat fails early.
- Used by ToolRouter enrichment and orchestrator **JIT guidance** (`build_descriptor_jit_guidance` in `orchestrator/core/turn_entry.rs`).

## Specs (`tools/specs.rs`)

Only holds embedded descriptor strings—no runtime tool loading from disk.

## Routing phrases (`tools/routing_phrases.rs`)

Compile-time **`fallback_triggers(tool_name)`** strings used when a tool has **no** `routing_hints` in its embedded TOML descriptor. Keeps ToolRouter embedding text and the slim phrase compendium aligned without duplicating every tool inside `tool_router.rs`.

**Slim phrase map:** `ContextAssembler::assemble_slim_tool_map` emits `[FCP_TOOL_PHRASE_MAP]` from the same per-tool phrasing (descriptor `routing_hints` else `fallback_triggers`). When **`db:find_connections`** or any **`calendar:*`** tool appears in the allowed tool list, the assembler also appends **`[SESSION_REFERENCE_TIME]`** (wall clock + default calendar year) so RFC3339 fields (`when`, `time_min` / `time_max`, `start_datetime` / `end_datetime`) need not guess the year—see `tools/clock/now.rs` and `orchestrator/context/assembler.rs`.

## Adding a tool (agent checklist)

1. Implement `Tool` in appropriate `tools/<area>/`.
2. Register in `executive/chat_session.rs` `Gatekeeper::register` (central chat bootstrap).
3. Add embedded descriptor TOML in `specs.rs` and wire into `DESCRIPTOR_TOMLS`.
4. If the tool should embed-route without full TOML hints, add a `fallback_triggers` arm in `routing_phrases.rs`.
5. If models often use wrong arg names, extend **`normalize_tool_args`** in `gatekeeper.rs`.
6. Run `cargo test`.
