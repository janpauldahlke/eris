# Vision and media catalog

Optional **image understanding** and **long-term image recall** for Eris:

- **`vision:see`** — multimodal describe via `llama-server`
- **`media:catalog` / `media:meta`** — structured **`40_MEDIA`** cards indexed in Qdrant (text only)
- **`vision:display`** — inline image preview in **web chat**

Users attach images in **web chat** or **Discord**; the model calls tools through the gatekeeper.

**Not available on the Ollama backend.** Requires **`llm_backend = "LlamaCpp"`**, a compatible **chat GGUF + mmproj**, and a recent **`llama-server`** build.

---

## Two-tier storage

| Tier | Path | Purpose |
| ---- | ---- | ------- |
| **Blob store** | `99_USER_UPLOADED/images/{sha256}.jpg` | Content-addressed JPEG bytes; never embedded in Qdrant |
| **Catalog cards** | `40_MEDIA/{content_hash}/media.json` | Title, description, tags, notes, file path — **text embedded in Qdrant** |

New uploads are normalized to JPEG under `[vision].upload_dir` (default `99_USER_UPLOADED/images/`). When the user asks to **remember** an image, the agent runs **`vision:see`** then **`media:catalog`**, which writes or updates the card under `40_MEDIA/`. **`memory:query`** can recall catalog text; **`vision:display`** shows pixels inline in the web UI.

**v1:** `media:catalog` supports **images only** (JPEG under the vision upload dir).

---

## Requirements

| Piece | Notes |
| ----- | ----- |
| Backend | **`LlamaCpp` only** — `vision.enabled` is rejected at startup for Ollama |
| `llama-server` | Recent **llama.cpp** (e.g. **b9493+**; Gemma 4 needs **`gemma4uv`** projector support — older builds fail with `unknown projector type: gemma4uv`) |
| Chat model | Multimodal-capable GGUF (validated in production: **Gemma 4 12B** + Unsloth mmproj) |
| mmproj | Separate **`.gguf`** projector; path via `llama_cpp.mmproj_path` |
| VRAM | Chat weights **plus** mmproj KV; large `num_ctx` + 12B may need **60–120s** load — raise `ready_timeout_secs` |
| Discord | `[discord] enabled` + bot token; images in the **listen channel** are downloaded and normalized like web uploads |
| Qdrant | **`40_MEDIA/`** ingest runs only when `[vision] enabled = true` (boot ingest + live re-index watch) |

Other multimodal GGUF families may work if llama.cpp supports their projector type and you supply the matching mmproj — treat as **experimental** until you verify load + `vision:see` on your stack.

---

## Configuration

Master switch — when `false`, Eris skips mmproj spawn, web upload routes, **`40_MEDIA`** Qdrant ingest, tool registration, and rejects image ingress:

```toml
llm_backend = "LlamaCpp"

[vision]
enabled = true
# upload_dir = "99_USER_UPLOADED/images"   # default
# target_max_px = 896
# max_upload_bytes = 1048576
# max_output_bytes = 2097152
# jpeg_quality = 85

[llama_cpp]
home = "/path/to/llama.cpp/build"
chat_model_path = "/path/to/chat.gguf"
mmproj_path = "/path/to/mmproj-F16.gguf"
# media_path defaults to vault root (for file:// paths in vision:see)
ready_timeout_secs = 120
```

Eris spawns chat `llama-server` with **`--mmproj`** and **`--media-path`** only when `[vision] enabled = true`.

---

## User flows

### Describe only (“what do you see?”)

1. Attach image in web or Discord.
2. Model calls **`vision:see`** with the vault-relative path from the attachment hint.
3. Reply in prose from the description — **no** `media:catalog` unless the user asked to remember.

### Remember (“remember this”)

1. **`vision:see`** on the attachment path.
2. **`media:catalog`** with `relative_path` + `description` from step 1.
3. **Title:** agent invents a short label from what it saw — **not** the user’s command phrase (“remember this”). Omit `title` when the description is enough; the tool derives one from the first sentence.
4. Optional: `tags`, `user_notes` if the user gave extra context.

After **`vision:see`** succeeds on a remember intent, the orchestrator injects **`[FCP MEDIA — CATALOG NEXT]`** so the model does not stop at describe-only.

### Recall and show

1. **`memory:query`** (or turn-start prefetch) for “colleague photo”, “fish truck”, etc. — hits `40_MEDIA` text in Qdrant.
2. **`vision:display`** with the blob `relative_path` from the card — web UI renders inline via `GET /api/vision/preview/{filename}`.

### Web (`eris chat --web`)

1. Drop an image on the compose area (or paste from clipboard where the browser allows).
2. Add a question in the same message (e.g. “what do you see?” or “remember this”).
3. Eris normalizes to JPEG under `99_USER_UPLOADED/images/`, hints the model with the vault path, and the model calls the appropriate vision/media tools.

Large camera JPEGs are pre-compressed in the browser; the server also steps down quality/size if needed. **HEIC/HEIF** is not supported in web upload — export as JPEG first.

### Discord (optional sidecar)

1. Post an image (± caption) in the configured listen channel.
2. The sidecar downloads the first image attachment from Discord CDN, runs the same normalize pipeline, and queues `UserIngress` with `image` set.
3. Same tool path as web; replies are **text only** in Discord (no inline `vision:display`).

Image-only messages are accepted (`display` shows as `(image attachment)` in the transcript).

---

## Tools

| Tool | Args (primary) | When |
| ---- | -------------- | ---- |
| **`vision:see`** | `relative_path`, optional `prompt` | Describe an attached or uploaded image |
| **`media:catalog`** | `relative_path`, `description`, optional `title`, `tags`, `user_notes` | User asked to remember/save/catalog |
| **`media:meta`** | `relative_path` or `content_hash` + patch fields | Update an existing card |
| **`vision:display`** | `relative_path` | User asked to show/display a known image (web inline) |

**Gatekeeper:** vision tools allowed in Chat / Reflect / Idle / Recover when vision is enabled. **`media:catalog`** and **`media:meta`** register always; vision-gated tools require `[vision] enabled`.

**Arg aliases:** the gatekeeper maps `path` and `file_path` → `relative_path` before JSON Schema validation for all four tools above (common model mistake).

**Routing hints:** see `routing_phrases.rs` / `specs.rs` and the [README phrase compendium](../../README.md#natural-language--tool-routing-phrase-compendium).

**Skill:** `media-catalog-workflow` under `10_Topology/skills/` when JIT guidance selects vision/media tools.

Check status: **`system:health`** includes a `vision` section (`enabled`, `upload_dir`, `mmproj_path`).

---

## Troubleshooting

| Symptom | Likely cause |
| ------- | ------------- |
| `unknown projector type: gemma4uv` | Rebuild llama.cpp from current master |
| `llama-chat failed to become ready within 30s` | Model+mmproj still loading — increase `ready_timeout_secs`; ensure no stale server on 8090 |
| `llama-server already running on port` (no mmproj) | Kill external `llama-server` on 8090/8091 so Eris spawns with vision flags |
| Web upload fails / 413 | File over `max_upload_bytes` — use smaller JPEG or raise limit |
| Discord image ignored | `vision.enabled = false`, or download/normalize failed — see `fcp.discord.image_ingest_failed` in logs |
| Orchestrator fatal on first image turn | Skill markdown missing YAML frontmatter under `10_Topology/skills/` |
| “Remember this” only describes, no card | Model skipped `media:catalog` — check logs for `[FCP MEDIA — CATALOG NEXT]`; re-prompt or say “catalog this photo” |
| Schema validation failed on `path` | Pre-polish builds — rebuild; gatekeeper should coerce `path`/`file_path` to `relative_path` |
| `memory:query` misses a cataloged image | `[vision] enabled = false` skips `40_MEDIA` ingest; or card missing — check `40_MEDIA/{hash}/media.json` on disk |
| Display works but no inline image in web | Not web UI, or `vision:display` result missing `display: true` — see orchestrator `AssistantImage` event |

---

## Related docs

- [LLAMA_CPP_SETUP.md](LLAMA_CPP_SETUP.md) — build `llama-server`, ports, GBNF
- [README phrase compendium](../../README.md#natural-language--tool-routing-phrase-compendium) — vision/media routing rows
- Architecture: [04_MEMORY_SUBSYSTEM.md](../updated_architecture/04_MEMORY_SUBSYSTEM.md) (`40_MEDIA` ingest), [05_TOOLS_GATEKEEPER_DESCRIPTORS.md](../updated_architecture/05_TOOLS_GATEKEEPER_DESCRIPTORS.md), [06_UI_TELEMETRY_OPERATIONS.md](../updated_architecture/06_UI_TELEMETRY_OPERATIONS.md) (`AssistantImage`)
