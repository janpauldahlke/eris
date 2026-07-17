# Eris tool roster

Curated inventory of gatekeeper tools (and a few non-tool surfaces). Mirrors the family catalog in [`src/ui/web/tools_config_schema.rs`](../src/ui/web/tools_config_schema.rs) (`FAMILIES`) plus **Document RAG**, which registers in chat but is not yet on the web Tools console.

Optional families only appear when config / credentials allow registration — see [`src/tools/registration.rs`](../src/tools/registration.rs) and [`src/executive/chat_session.rs`](../src/executive/chat_session.rs).

For natural-language routing hints (how to *phrase* requests), see [REFERENCE.md § Natural language → tool routing](REFERENCE.md#natural-language--tool-routing-phrase-compendium). To add a tool: [HOW_TO/ADDING_A_TOOL.md](HOW_TO/ADDING_A_TOOL.md).

## Core (always on the chat path)

| Family | Tools | Summary |
| --- | --- | --- |
| **Vault** | `vault:read` · `vault:write` · `vault:list` · `vault:search` · `vault:taglist` | Read, write, list, search, and tag-index markdown under the vault |
| **Skills** | `skills:list` · `skills:read` · `skills:create` | Topology skills under `10_Topology/skills/` |
| **Memory** | `memory:stage` · `memory:staged_list` · `memory:commit` · `memory:commit_all` · `memory:query` | Ephemeral staging, commit to vault/Qdrant, semantic recall |
| **Agenda** | `agenda:push` · `agenda:list` · `agenda:remind_at` · `agenda:remind_self` · `agenda:complete` · `agenda:remove` | Tasks, reminders, self-driven follow-ups |
| **Clock** | `clock:now` · `clock:timer` · `clock:alarm` | Wall time, timers, standalone alarms |
| **System** | `system:health` | Runtime health snapshot |
| **Media catalog** | `media:catalog` · `media:meta` | Remembered images under `40_MEDIA/` |

## Opt-in / hardware-gated

| Family | Tools | When |
| --- | --- | --- |
| **Web** | `web:fetch` · `web:find` · `web:search` | browser39 + allowlist; `web:search` needs `[web] search_enabled` |
| **News** | `news:today` | `news_today_enabled` |
| **Weather** | `weather:current` · `weather:forecast` | `weather_enabled` (Open-Meteo) |
| **Wikipedia** | `wiki:summary` | `wiki_enabled` |
| **Trains (DB)** | `db:find_connections` | `db_rest_enabled` |
| **Document RAG** | `doc:ingest` · `doc:query` · `doc:read` · `doc:list` · `doc:delete` | `[document_rag] enabled` + document store available |
| **Google Workspace** | `mail:check` · `mail:read` · `mail:digest` · `mail:delete` · `mail:move` · `mail:write` · `calendar:list` · `calendar:get` · `calendar:create` · `calendar:update` · `calendar:delete` | `[google] enabled` + service-account credentials |
| **Moltbook** | `moltbook:register` · `moltbook:status` · `moltbook:home` · `moltbook:feed` · `moltbook:search` · `moltbook:comments` · `moltbook:comment` · `moltbook:vote` · `moltbook:post` · `moltbook:verify` · `moltbook:notifications_read` · `moltbook:dm` | `[moltbook] enabled` (+ API key for authenticated tools) |
| **Vision** | `vision:see` · `vision:display` | llama.cpp + `[vision] enabled` + multimodal GGUF/mmproj |

## Surfaces that are not gatekeeper tools

| Surface | Notes |
| --- | --- |
| **Audio** | Web mic / upload → STT before the orchestrator turn (`[audio] enabled`) |
| **Discord** | Optional Serenity sidecar sharing the live session |

## Counts (ballpark)

Core gatekeeper tools above: **29**. With every optional family registered (including Document RAG and full Moltbook/Google): on the order of **~60+** named tools — the exact set is whatever `gatekeeper.registered_tool_names()` returns for your config.
