# Voice ingress (STT before orchestrator turn)

Optional **speech-to-text ingress** for Eris: users attach audio or record from the microphone in **web chat**; Eris transcribes via Gemma 4 + `llama-server`, then treats the transcript as **plain user text** (same tool routing as typing).

**Not available on the Ollama backend.** Requires **`llm_backend = "LlamaCpp"`**, the same **chat GGUF + mmproj** as vision, **`ffmpeg`** on PATH, and a recent **`llama-server`** with `input_audio` support.

Unlike **`vision:see`**, there is **no model-callable audio tool** — transcription runs automatically before the orchestrator turn so spoken commands like *"perform a system health check"* route to `system:health` normally.

---

## Requirements

| Piece | Notes |
| ----- | ----- |
| Backend | **`LlamaCpp` only** |
| Model | **Gemma 4 12B** (or other omni GGUF with audio in llama.cpp) — same files as vision |
| mmproj | Same `llama_cpp.mmproj_path` as vision |
| `ffmpeg` | Required for mp3/webm/m4a normalization (`apt install ffmpeg`) |
| Pre-built WAV | 16 kHz mono WAV may pass through without ffmpeg |

No separate HuggingFace download is needed if you already run Gemma 4 12B + mmproj for vision.

---

## Local CLI: `ffmpeg` (required)

Eris does **not** ship ffmpeg. Voice ingress shells out to the **`ffmpeg` binary on your PATH** when normalizing browser recordings (WebM/Opus from the mic) and most uploaded formats.

Install once on the host where you run `eris chat`:

**Debian / Ubuntu:**

```bash
sudo apt-get update
sudo apt-get install -y ffmpeg
ffmpeg -version
```

**macOS (Homebrew):**

```bash
brew install ffmpeg
ffmpeg -version
```

Verify from the same shell you use to start Eris:

```bash
which ffmpeg
```

If ffmpeg is missing, uploads fail with `ffmpeg not found on PATH` in the web UI. Pre-built **16 kHz mono WAV** files may skip ffmpeg (fast path), but **mic capture always needs ffmpeg** (MediaRecorder output is WebM).

Optional override: set `FFMPEG=/path/to/ffmpeg` if the binary is not on PATH.

---

## Configuration

```toml
llm_backend = "LlamaCpp"

[audio]
enabled = true
# upload_dir = "99_USER_UPLOADED/audio"   # default
# max_upload_bytes = 10485760
# max_duration_secs = 30
# transcription_prompt = "Transcribe the speech verbatim. Output only the spoken words."

[llama_cpp]
mmproj_path = "/path/to/mmproj-F16.gguf"
```

Eris spawns chat `llama-server` with **`--mmproj`**, **`--media-path`**, and **`--jinja`** when **either** `[vision] enabled` or `[audio] enabled`.

---

## User flow (web)

1. Click the **Mic** button (small circle beside compose) — click again to stop and **send immediately** (no Enter).
2. Optional: type a caption in the compose box before stopping — merged as **transcript + caption**.
3. On send, Eris shows `[ui] Transcribing voice…`, transcribes, then runs the orchestrator turn.

Normalized WAV files under `upload_dir` are **deleted on chat exit** by default (`cleanup_uploads_on_chat_exit = true`).

---

## Vault storage

Normalized WAV files: `99_USER_UPLOADED/audio/{uuid}.wav`

---

## Troubleshooting

| Symptom | Likely cause |
| ------- | ------------- |
| `ffmpeg not found on PATH` | Install ffmpeg |
| `Voice transcription failed` | llama.cpp too old, or server not running with mmproj |
| Empty transcript | Silence clip, or thinking channel — we send `enable_thinking: false` |
| mp3/webm fails, wav works | ffmpeg missing or format unsupported |

---

## Related

- [VISION.md](VISION.md) — image path (different model-call pattern)
- [LLAMA_CPP_SETUP.md](LLAMA_CPP_SETUP.md)
