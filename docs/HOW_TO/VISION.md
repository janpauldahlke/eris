# Vision (multimodal `vision:see`)

Optional **image understanding** for Eris: users attach images in **web chat** or **Discord**; the model calls **`vision:see`** to describe them via a multimodal `llama-server`.

**Not available on the Ollama backend.** Requires **`llm_backend = "LlamaCpp"`**, a compatible **chat GGUF + mmproj**, and a recent **`llama-server`** build.

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

Other multimodal GGUF families may work if llama.cpp supports their projector type and you supply the matching mmproj — treat as **experimental** until you verify load + `vision:see` on your stack.

---

## Configuration

Master switch — when `false`, Eris skips mmproj spawn, web upload routes, tool registration, and rejects image ingress:

```toml
llm_backend = "LlamaCpp"

[vision]
enabled = true
# upload_dir = "99_USER_UPLOADED/images"   # default
# target_max_px = 896
# max_upload_bytes = 10485760
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

### Web (`eris chat --web`)

1. Drop an image on the compose area (or paste from clipboard where the browser allows).
2. Add a question in the same message (e.g. “what do you see?”).
3. Eris normalizes to JPEG under `99_USER_UPLOADED/images/{uuid}.jpg`, hints the model with the vault path, and the model calls **`vision:see`**.

Large camera JPEGs are pre-compressed in the browser; the server also steps down quality/size if needed. **HEIC/HEIF** is not supported in web upload — export as JPEG first.

### Discord (optional sidecar)

1. Post an image (± caption) in the configured listen channel.
2. The sidecar downloads the first image attachment from Discord CDN, runs the same normalize pipeline, and queues `UserIngress` with `image` set.
3. Same **`vision:see`** path as web; replies are **text only** in Discord.

Image-only messages are accepted (`display` shows as `(image attachment)` in the transcript).

---

## Tool: `vision:see`

- **Args:** `relative_path` (under `[vision].upload_dir`), optional `prompt`
- **Gatekeeper:** allowed in Chat / Reflect / Idle / Recover when vision is enabled
- **Routing hints:** “describe image”, “what do you see”, “attached image”, … (see `routing_phrases.rs` / `specs.rs`)
- **Skill:** `vision-upload-workflow` under `10_Topology/skills/` when JIT guidance selects the tool

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

---

## Related docs

- [LLAMA_CPP_SETUP.md](LLAMA_CPP_SETUP.md) — build `llama-server`, ports, GBNF
- [README phrase compendium](../../README.md#natural-language--tool-routing-phrase-compendium) — `vision:see` routing row
- Architecture: [05_TOOLS_GATEKEEPER_DESCRIPTORS.md](../updated_architecture/05_TOOLS_GATEKEEPER_DESCRIPTORS.md), [06_UI_TELEMETRY_OPERATIONS.md](../updated_architecture/06_UI_TELEMETRY_OPERATIONS.md)
