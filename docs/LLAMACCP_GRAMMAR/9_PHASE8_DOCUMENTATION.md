# Phase 8 — Documentation and Operator Manual

**Depends on:** All prior phases (documents what was built)
**Unlocks:** Nothing (terminal phase)
**Can run in parallel with:** Phase 6, Phase 7 (documentation for stable phases can be written early)
**Estimated scope:** ~400 lines of documentation

---

## 8.1 — Goal

Provide operator-facing documentation for the llama.cpp backend: how to build llama.cpp, obtain GGUF models, configure Eris, and troubleshoot common issues.

---

## 8.2 — Files to Create/Modify

### 8.2.1 New: `docs/LLAMA_CPP_SETUP.md`

Dedicated setup guide. This is the primary document an operator reads to get llama.cpp working with Eris.

### 8.2.2 Modified: `docs/OPERATOR_MANUAL.md`

Add a "Backend Selection" section with a pointer to the dedicated guide.

---

## 8.3 — `docs/LLAMA_CPP_SETUP.md` Outline

### 8.3.1 Prerequisites

- A working Rust toolchain (for building Eris — already a requirement)
- CMake, a C/C++ compiler (for building llama.cpp)
- A GGUF model file for chat
- A GGUF model file for embeddings
- Qdrant running (same as Ollama setup)

### 8.3.2 Building llama.cpp

Platform-specific instructions:

**macOS (Apple Silicon with Metal):**
```bash
git clone https://github.com/ggerganov/llama.cpp.git
cd llama.cpp
cmake -B build -DGGML_METAL=ON
cmake --build build --config Release -j $(sysctl -n hw.ncpu)
```

Verify: `./build/bin/llama-server --help`

**Linux (NVIDIA CUDA):**
```bash
git clone https://github.com/ggerganov/llama.cpp.git
cd llama.cpp
cmake -B build -DGGML_CUDA=ON
cmake --build build --config Release -j $(nproc)
```

Requires: CUDA toolkit installed, `nvcc` on PATH.

**Linux / macOS (CPU only):**
```bash
cmake -B build
cmake --build build --config Release -j $(nproc)
```

Works everywhere but slow for large models.

### 8.3.3 Obtaining GGUF Models

**Chat model (recommended):**
- Qwen 2.5 14B Instruct Q4_K_M: good balance of quality and speed
- Download via `huggingface-cli`:
  ```bash
  pip install huggingface-hub
  huggingface-cli download Qwen/Qwen2.5-14B-Instruct-GGUF \
      qwen2.5-14b-instruct-q4_k_m.gguf \
      --local-dir ./models
  ```
- Or direct download from HuggingFace web UI

**Embedding model (recommended):**
- nomic-embed-text v1.5 Q8_0: same model Ollama uses, same 768-dim vectors
- Download:
  ```bash
  huggingface-cli download nomic-ai/nomic-embed-text-v1.5-GGUF \
      nomic-embed-text-v1.5.Q8_0.gguf \
      --local-dir ./models
  ```

**VRAM requirements table:**

| Model | Quantization | VRAM (full offload) | RAM (CPU) |
|-------|-------------|--------------------:|----------:|
| Qwen 2.5 7B | Q4_K_M | ~5 GB | ~6 GB |
| Qwen 2.5 14B | Q4_K_M | ~9 GB | ~10 GB |
| Qwen 2.5 14B | Q8_0 | ~16 GB | ~17 GB |
| nomic-embed-text 1.5 | Q8_0 | ~0.2 GB | ~0.3 GB |

### 8.3.4 Eris Configuration

**First-time setup (ignition via `eris chat` in a fresh directory):**
```bash
mkdir my-vault && cd my-vault
eris chat
# Ignition fires automatically (no seal file found)
# Select "llama.cpp" at the Backend prompt
# Follow prompts for llama.cpp home, model paths, etc.
# Chat session starts after ignition completes
```

**Manual `config.toml` configuration:**

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

Context length for managed `llama-server` is top-level **`num_ctx`** (not under `[llama_cpp]`).

**Field reference:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `home` | path | — | Path to `llama.cpp/build` (must contain `bin/llama-server`) |
| `chat_server_url` | URL | `http://127.0.0.1:8090` | Chat server endpoint |
| `embed_server_url` | URL | `http://127.0.0.1:8091` | Embedding server endpoint |
| `chat_model_path` | path | — | Full path to chat GGUF |
| `embed_model_path` | path | — | Full path to embed GGUF |
| `n_gpu_layers` | int | 0 | Layers offloaded to GPU. `99` = all. `0` = CPU only. |
| `ready_timeout_secs` | int | 30 | Max seconds to wait for server readiness |

### 8.3.5 Running

```bash
eris chat
# Eris spawns llama-server (chat + embed), waits for readiness, starts chat
```

To manage llama-server yourself (external mode):

```bash
# Terminal 1: Start chat server manually
/path/to/llama.cpp/build/bin/llama-server \
    --model /models/qwen2.5-14b.gguf \
    --port 8090 --ctx-size 32768 --n-gpu-layers 99

# Terminal 2: Start embed server manually
/path/to/llama.cpp/build/bin/llama-server \
    --model /models/nomic-embed-text.gguf \
    --port 8091 --embedding --ctx-size 8192

# Terminal 3: Start Eris (detects running servers, skips spawn)
eris chat
```

### 8.3.6 Switching Backends

To switch an existing vault from Ollama to llama.cpp:

1. Edit `.fcp/config.toml`
2. Add `llm_backend = "LlamaCpp"` and the `[llama_cpp]` section
3. If using the same embedding model (nomic-embed-text), Qdrant collections are compatible — no migration needed
4. If using a different embedding model, delete the Qdrant collection and re-index: `eris reindex` (if available) or delete collection via Qdrant API and restart

To switch back to Ollama: set `llm_backend = "Ollama"` (or remove the key entirely).

### 8.3.7 Troubleshooting

**Problem: "llama-server binary not found"**
- Check that `home` points to the build directory containing `bin/llama-server`
- Run `ls {home}/bin/llama-server` to verify

**Problem: "llama-server failed to start within timeout"**
- Large models can take 30-60 seconds to load
- Increase `ready_timeout_secs` to 120
- Check VRAM: `nvidia-smi` (Linux) or Activity Monitor (macOS)
- If model exceeds VRAM, reduce `n_gpu_layers` or use a smaller quantization

**Problem: Port conflict**
- Another process is using 8090 or 8091
- Change ports in `config.toml` (`chat_server_url`, `embed_server_url`)
- Check: `lsof -i :8090`

**Problem: "Embedding dimension mismatch"**
- The embed GGUF model produces different-sized vectors than the existing Qdrant collection
- Solution: use the same embedding model dimension, or delete and recreate the Qdrant collection

**Problem: GPU layers / VRAM**
- Start with `n_gpu_layers = 0` (CPU only) to verify the setup works
- Gradually increase until you hit VRAM limits
- `n_gpu_layers = 99` means "offload everything" — reduce if OOM

**Problem: Slow generation**
- Verify GPU offload is working: check `llama-server` startup log for "offloaded N/M layers to GPU"
- On Apple Silicon: ensure Metal is enabled (`-DGGML_METAL=ON` during build)
- On Linux: ensure CUDA is enabled and `nvidia-smi` shows the process

**Problem: "Grammar parse error" from llama-server**
- This indicates a bug in the GBNF grammar compilation (Phase 4/7)
- File an issue with the grammar string (logged at startup)
- Workaround: the Ollama backend doesn't use grammar — switch back temporarily

---

## 8.4 — `docs/OPERATOR_MANUAL.md` Updates

Add a new section after the existing content:

```markdown
## Backend Selection

Eris supports two LLM backends:

- **Ollama** (default): Manages models via Ollama's API. Easiest setup.
- **llama.cpp**: Direct GGUF model inference via llama-server. More control, GBNF grammar enforcement for structured output.

See [LLAMA_CPP_SETUP.md](LLAMA_CPP_SETUP.md) for llama.cpp installation and configuration.

Backend is selected during first-run ignition (`eris chat` in a fresh vault directory) or by setting `llm_backend` in `.fcp/config.toml`.
```

---

## 8.5 — Acceptance Criteria

- [ ] A new operator can follow `LLAMA_CPP_SETUP.md` from scratch and get Eris running with llama.cpp
- [ ] Build instructions work on macOS (Metal) and Linux (CUDA)
- [ ] Config reference covers all `[llama_cpp]` fields
- [ ] Troubleshooting covers the 7 most common failure modes
- [ ] `OPERATOR_MANUAL.md` links to the new guide
- [ ] No outdated or misleading information
