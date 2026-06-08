# llama.cpp Backend — Setup Guide

Eris supports two LLM backends: **Ollama** (default, easiest) and **llama.cpp** (direct GGUF inference via `llama-server`, GBNF grammar enforcement for structured output). This guide covers the llama.cpp path.

---

## 1. Prerequisites

- Rust toolchain (for building Eris — you already have this)
- CMake + C/C++ compiler (for building llama.cpp)
- A GGUF chat model
- A GGUF embedding model
- Qdrant running (same as the Ollama setup)
- **`ffmpeg` on PATH** — required when **`[audio] enabled = true`** (voice/mic ingress). Eris invokes it as a local CLI subprocess; it is not bundled. See **[AUDIO.md](AUDIO.md#local-cli-ffmpeg-required)** (`apt install ffmpeg` / `brew install ffmpeg`)

---

## 2. Building llama.cpp

Eris does **not** compile llama.cpp — you build it separately and point Eris at the build directory.

**macOS (Apple Silicon / Metal):**

```bash
git clone https://github.com/ggerganov/llama.cpp.git
cd llama.cpp
cmake -B build -DGGML_METAL=ON
cmake --build build --config Release -j $(sysctl -n hw.ncpu)
```

**Linux (NVIDIA CUDA):**

```bash
git clone https://github.com/ggerganov/llama.cpp.git
cd llama.cpp
cmake -B build -DGGML_CUDA=ON
cmake --build build --config Release -j $(nproc)
```

Requires CUDA toolkit installed and `nvcc` on PATH.

**Linux / macOS (CPU only):**

```bash
git clone https://github.com/ggerganov/llama.cpp.git
cd llama.cpp
cmake -B build
cmake --build build --config Release -j $(nproc)
```

Works everywhere but slow for larger models.

**Verify:** `./build/bin/llama-server --help` should print usage.

---

## 3. Obtaining GGUF Models

Download via `huggingface-cli` or the HuggingFace web UI.

```bash
pip install huggingface-hub
```

**Chat model (recommended — Qwen 2.5 14B Instruct Q4_K_M):**

```bash
huggingface-cli download Qwen/Qwen2.5-14B-Instruct-GGUF \
    qwen2.5-14b-instruct-q4_k_m.gguf \
    --local-dir ./models
```

**Embedding model (recommended — nomic-embed-text v1.5 Q8_0):**

```bash
huggingface-cli download nomic-ai/nomic-embed-text-v1.5-GGUF \
    nomic-embed-text-v1.5.Q8_0.gguf \
    --local-dir ./models
```

Using `nomic-embed-text` for embeddings means your Qdrant collections are compatible with the Ollama path (same 768-dim vectors) — no migration needed when switching backends.

**VRAM requirements:**

| Model | Quantization | VRAM (full offload) | RAM (CPU) |
|-------|-------------|--------------------:|----------:|
| Qwen 2.5 7B | Q4_K_M | ~5 GB | ~6 GB |
| Qwen 2.5 14B | Q4_K_M | ~9 GB | ~10 GB |
| Qwen 2.5 14B | Q8_0 | ~16 GB | ~17 GB |
| nomic-embed-text 1.5 | Q8_0 | ~0.2 GB | ~0.3 GB |

---

## 4. Configuration

### First-time setup (ignition wizard)

```bash
mkdir my-vault && cd my-vault
eris chat
```

Ignition detects no seal file and walks you through setup. Select **llama.cpp** at the backend prompt, then provide paths to the llama.cpp build directory and your GGUF models.

### Manual `config.toml`

If you already have a vault, edit `.fcp/config.toml`:

```toml
llm_backend = "LlamaCpp"

[llama_cpp]
home = "/path/to/llama.cpp/build"
chat_server_url = "http://127.0.0.1:8090"
embed_server_url = "http://127.0.0.1:8091"
chat_model_path = "/path/to/models/qwen2.5-14b-instruct-q4_k_m.gguf"
embed_model_path = "/path/to/models/nomic-embed-text-v1.5.Q8_0.gguf"
n_gpu_layers = 99
ready_timeout_secs = 60
```

Context length for the managed `llama-server` comes from the top-level `num_ctx` key (not under `[llama_cpp]`).

### Field reference

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `home` | path | — | Path to `llama.cpp/build` (must contain `bin/llama-server`) |
| `chat_server_url` | URL | `http://127.0.0.1:8090` | Chat server endpoint |
| `embed_server_url` | URL | `http://127.0.0.1:8091` | Embedding server endpoint |
| `chat_model_path` | path | — | Full path to chat GGUF |
| `embed_model_path` | path | — | Full path to embed GGUF |
| `n_gpu_layers` | int | 0 | Layers offloaded to GPU. `99` = all. `0` = CPU only. |
| `ready_timeout_secs` | int | 30 | Max seconds to wait for server readiness |
| `mmproj_path` | path | — | Multimodal projector GGUF; required when `[vision] enabled = true` |
| `media_path` | path | vault root | `--media-path` for `file://` image paths in `vision:see` |

---

## 4b. Vision (optional multimodal)

Image understanding is **only** on the llama.cpp path (`vision:see`). Ollama backend does not register the tool or spawn mmproj.

1. Use a **multimodal chat GGUF** and matching **mmproj** (tested: Gemma 4 12B + Unsloth `mmproj-F16.gguf`).
2. Build **recent llama.cpp** — Gemma 4’s `gemma4uv` projector needs **b9493+**; older `llama-server` builds fail at mmproj load.
3. Enable in config:

```toml
[vision]
enabled = true

[llama_cpp]
mmproj_path = "/path/to/mmproj-F16.gguf"
ready_timeout_secs = 120
```

4. **Web:** `eris chat --web` — drop image + question in compose area.
5. **Discord:** optional sidecar downloads channel image attachments into the same vault upload folder.

Operator guide: **[VISION.md](VISION.md)**.

---

## 4c. Voice ingress (optional STT)

Speech-to-text runs **before** the orchestrator turn (spoken commands route like typed text). Requires the same mmproj stack as vision plus **`ffmpeg` installed locally** — see **[AUDIO.md](AUDIO.md)**.

```toml
[audio]
enabled = true
```

Web chat: small **Mic** button beside compose — click to record, click again to send.

---

## 5. Running

**Managed mode (default):** Eris spawns and manages the llama-server processes.

```bash
cd /path/to/your/vault
eris chat
```

Eris starts two `llama-server` instances (chat on port 8090, embeddings on port 8091), waits for readiness via `/health`, then starts the session. Both servers are torn down on exit.

**External mode:** Run llama-server yourself if you want persistent servers or custom flags.

```bash
# Terminal 1 — chat server
/path/to/llama.cpp/build/bin/llama-server \
    --model /models/qwen2.5-14b.gguf \
    --port 8090 --ctx-size 32768 --n-gpu-layers 99

# Terminal 2 — embed server
/path/to/llama.cpp/build/bin/llama-server \
    --model /models/nomic-embed-text.gguf \
    --port 8091 --embedding --ctx-size 8192

# Terminal 3 — Eris detects running servers and skips spawn
eris chat
```

---

## 6. Switching Backends

**Ollama to llama.cpp:**

1. Edit `.fcp/config.toml`
2. Set `llm_backend = "LlamaCpp"` and add the `[llama_cpp]` section
3. If using `nomic-embed-text` for embeddings in both backends, Qdrant collections are compatible — no re-indexing needed
4. If using a different embedding model, delete the Qdrant collection and re-index

**llama.cpp to Ollama:** Set `llm_backend = "Ollama"` (or remove the key — Ollama is the default).

---

## 7. GBNF Grammar (what it does for you)

When using the llama.cpp backend, Eris compiles a **GBNF grammar** at session start that constrains the model's output to valid FCP protocol JSON. This means:

- The model **cannot** produce malformed JSON — parse failures are structurally impossible
- Tool names are constrained to the registered set
- Tool arguments are constrained per-tool to match each tool's JSON Schema
- The recovery path is simpler: no JSON-repair retries, only schema-level retries in natural language

This is the main advantage over the Ollama path, where the model can (and occasionally does) produce invalid JSON that triggers recovery loops.

---

## 8. Troubleshooting

**"llama-server binary not found"**
- Verify `home` points to the build directory containing `bin/llama-server`
- Run `ls {home}/bin/llama-server`

**"llama-server failed to start within timeout"**
- Large models can take 30-60s to load; increase `ready_timeout_secs` to 120
- Check VRAM: `nvidia-smi` (Linux) or Activity Monitor (macOS)
- If the model exceeds VRAM, reduce `n_gpu_layers` or use a smaller quantization

**Port conflict**
- Another process is using 8090 or 8091
- Change ports in `config.toml` (`chat_server_url`, `embed_server_url`)
- Check: `lsof -i :8090`

**"Embedding dimension mismatch"**
- The embed model produces vectors with a different width than the existing Qdrant collection
- Use the same embedding model, or delete and recreate the Qdrant collection

**GPU layers / VRAM**
- Start with `n_gpu_layers = 0` (CPU only) to verify the setup works
- Gradually increase until you hit VRAM limits
- `99` means "offload everything" — reduce if OOM

**Slow generation**
- Verify GPU offload: check `llama-server` startup log for "offloaded N/M layers to GPU"
- Apple Silicon: ensure Metal is enabled (`-DGGML_METAL=ON` during build)
- Linux: ensure CUDA is enabled and `nvidia-smi` shows the process

**"Grammar parse error" from llama-server**
- Indicates a bug in the GBNF grammar compilation
- File an issue with the grammar string (logged at startup)
- Workaround: switch to the Ollama backend temporarily (`llm_backend = "Ollama"`)

**`unknown projector type: gemma4uv` (vision)**
- Your `llama-server` build predates Gemma 4 multimodal support — rebuild llama.cpp from current master (see [VISION.md](VISION.md))

**Vision enabled but chat server has no mmproj**
- Stale external `llama-server` on port 8090 — stop it so Eris spawns a managed instance with `--mmproj` / `--media-path`
