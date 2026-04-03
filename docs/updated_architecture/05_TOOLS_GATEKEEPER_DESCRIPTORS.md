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
  - **Chat:** most tools except `agenda:complete` (prevents completing before user turn semantics).
  - **Reflect:** tool-sandbox style set: memory, vault read/list, web artifact query, agenda mutations, clocks, weather, wiki, system health—not `vault:write`.
  - **Idle:** includes `vault:write`, `web:fetch`, `agenda:complete`, etc.
  - **Recover:** all registered (recovery pass).
- **`execute_tool`:** state check → JSON Schema validate (`jsonschema`) → `tool.execute`.
- **Recover empty result** → `ToolFault` semantic guard.

## Validation (`tools/validation.rs`)

Shared helpers (e.g. path mutability) used by vault tools.

## Tool families (by module)

| Module | Tools (examples) |
|--------|------------------|
| `tools/vault/` | `vault:read`, `vault:write`, `vault:list` |
| `tools/memory/` | `memory:stage`, `memory:staged_list`, `memory:commit`, `memory:commit_all`, `memory:query` |
| `tools/agenda/` | `agenda:push`, `agenda:list`, `agenda:remind_at`, `agenda:complete`, `agenda:remove` |
| `tools/web/` | `web:fetch`, `web:artifact_query` |
| `tools/system/` | `system:health` |
| `tools/clock/` | `clock:now`, `clock:timer`, `clock:alarm` |
| `tools/weather/` | `weather:current`, `weather:forecast` (Open-Meteo via `ApiHttpClient`) |
| `tools/wiki/` | `wiki:summary` (Wikipedia REST) |

Agenda and alarms persist JSON under `.fcp/tools/` (see `vault_layout`).

## Descriptors (`tools/descriptors.rs`, `tools/specs.rs`)

- **Embedded TOML** slices compiled into the binary (`DESCRIPTOR_TOMLS`).
- Per tool: `when_to_use`, `when_not_to_use`, `routing_hints`, good/bad examples.
- **`ToolDescriptorRegistry::load_embedded`** at startup; **must cover** every registered tool name or chat fails early.
- Used by ToolRouter enrichment and orchestrator **JIT guidance** (`build_descriptor_jit_guidance`).

## Specs (`tools/specs.rs`)

Only holds embedded descriptor strings—no runtime tool loading from disk.

## Adding a tool (agent checklist)

1. Implement `Tool` in appropriate `tools/<area>/`.
2. Register in `executive/router.rs` `Gatekeeper::register`.
3. Add embedded descriptor TOML in `specs.rs` and wire into `DESCRIPTOR_TOMLS`.
4. Extend `tool_router::enrich_for_routing` default hints if needed for new `name`.
5. Run `cargo test`.
