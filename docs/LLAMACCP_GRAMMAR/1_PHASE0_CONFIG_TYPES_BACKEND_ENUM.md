# Phase 0 — Config, Types, and Backend Enum

**Depends on:** Nothing (first phase)
**Unlocks:** Phase 1 (engine impl), Phase 2 (process mgmt)
**Estimated scope:** ~200 LOC new, ~60 LOC modified, ~8 tests

---

## 0.1 — Goal

Extend `AppConfig` and the ignition flow so the system knows **which backend is active**, where `llama-server` lives, and what ports/paths to use. No behavioral changes to runtime — Ollama path compiles and runs identically. This is pure plumbing: types, serialization, and the interactive setup prompt.

---

## 0.2 — New Types in `src/config.rs`

### 0.2.1 `LlmBackend` enum

Insert **above** the `AppConfig` struct definition (around line 217).

```rust
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
pub enum LlmBackend {
    #[default]
    Ollama,
    LlamaCpp,
}
```

**Rationale for `Default = Ollama`:** Existing vaults have no `llm_backend` field in their `config.toml`. TOML deserialization with `#[serde(default)]` must produce `Ollama` so they continue working without migration.

### 0.2.2 `LlamaCppConfig` struct

```rust
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct LlamaCppConfig {
    /// Path to the llama.cpp build directory (contains bin/llama-server).
    pub home: PathBuf,
    /// Host:port for the chat model llama-server instance.
    #[serde(default = "default_llamacpp_chat_server_url")]
    pub chat_server_url: String,
    /// Host:port for the embedding model llama-server instance.
    #[serde(default = "default_llamacpp_embed_server_url")]
    pub embed_server_url: String,
    /// GGUF model file for chat.
    pub chat_model_path: PathBuf,
    /// GGUF model file for embeddings.
    pub embed_model_path: PathBuf,
    /// Context window for the chat server (--ctx-size).
    #[serde(default = "default_llamacpp_ctx_size")]
    pub ctx_size: usize,
    /// GPU layers to offload (--n-gpu-layers); 0 = CPU only.
    #[serde(default)]
    pub n_gpu_layers: u32,
}
```

Default helper functions:

```rust
fn default_llamacpp_chat_server_url() -> String {
    "http://127.0.0.1:8090".into()
}
fn default_llamacpp_embed_server_url() -> String {
    "http://127.0.0.1:8091".into()
}
fn default_llamacpp_ctx_size() -> usize {
    8192
}
```

### 0.2.3 `AppConfig` additions

Add two fields to `AppConfig`:

```rust
/// Active LLM backend; existing vaults without this key default to Ollama.
#[serde(default)]
pub llm_backend: LlmBackend,

/// llama.cpp-specific config; `None` when backend is Ollama.
#[serde(default)]
pub llama_cpp: Option<LlamaCppConfig>,
```

**Ordering:** Place `llm_backend` right after `model_name` (line ~229) so backend fields group logically with model/engine fields. Place `llama_cpp` right after `ollama_low_vram`.

---

## 0.3 — Validation Helper on `AppConfig`

Add an `impl AppConfig` method:

```rust
/// Validate llama.cpp config when backend is LlamaCpp.
/// Returns Err if required paths are missing or llama-server binary not found.
pub fn validate_llamacpp_config(&self) -> Result<&LlamaCppConfig> {
    if self.llm_backend != LlmBackend::LlamaCpp {
        return Err(        FcpError::Config(
            "validate_llamacpp_config called but backend is not LlamaCpp".into(),
        ));
    }
    let lc = self.llama_cpp.as_ref().ok_or_else(|| {
        FcpError::Config("[llama_cpp] section required when llm_backend = LlamaCpp".into())
    })?;
    let server_bin = lc.home.join("bin").join("llama-server");
    if !server_bin.exists() {
        return Err(FcpError::Config(format!(
            "llama-server binary not found at {}",
            server_bin.display()
        )));
    }
    if !lc.chat_model_path.exists() {
        return Err(FcpError::Config(format!(
            "Chat GGUF not found: {}", lc.chat_model_path.display()
        )));
    }
    if !lc.embed_model_path.exists() {
        return Err(FcpError::Config(format!(
            "Embed GGUF not found: {}", lc.embed_model_path.display()
        )));
    }
    Ok(lc)
}
```

**Note:** This uses the existing `FcpError::Config(String)` variant (see `src/executive/error.rs`). No new error variant needed.

---

## 0.4 — TOML Serialization Shape

When written by ignition, the `config.toml` gains:

```toml
llm_backend = "LlamaCpp"

[llama_cpp]
home = "/Users/me/llama.cpp/build"
chat_server_url = "http://127.0.0.1:8090"
embed_server_url = "http://127.0.0.1:8091"
chat_model_path = "/models/qwen2.5-14b-instruct-q4_k_m.gguf"
embed_model_path = "/models/nomic-embed-text-v1.5.Q8_0.gguf"
ctx_size = 32768
n_gpu_layers = 99
```

For Ollama vaults: no `llm_backend` key at all (defaults to `Ollama`), no `[llama_cpp]` section.

---

## 0.5 — Ignition Flow Changes (`src/executive/ignition.rs`)

**Context:** There is no `eris init` subcommand. Ignition runs inline from `start_chat_session` when the vault seal file doesn't exist (`eris chat` in a fresh directory). This is unchanged — the backend selector is added to the existing `run_ignition_sequence` function.

### 0.5.1 New Backend Selection Prompt

After the existing "Agent Name" / "Your name" prompts, insert a new `Select`:

```
Backend: [Ollama, llama.cpp]
```

Use the same `inquire::Select` pattern as the existing model selector in `run_ignition_sequence`.

### 0.5.2 llama.cpp Sub-Prompts (only when LlamaCpp selected)

1. **llama.cpp home directory** — `Input` prompt with default `~/llama.cpp/build`. Validate that `{home}/bin/llama-server` exists. Re-prompt on failure with error message.
2. **Chat model GGUF path** — `Input` prompt. Validate file exists and ends in `.gguf`. Re-prompt on failure.
3. **Embed model GGUF path** — `Input` prompt with default suggestion `{home}/../models/nomic-embed-text-v1.5.Q8_0.gguf` (common convention). Validate exists.
4. **Context size** — `Input` with default `8192`. Parse as `usize`.
5. **GPU layers** — `Input` with default `99` (offload all). Parse as `u32`.

### 0.5.3 Config Write

Populate `AppConfig.llm_backend = LlmBackend::LlamaCpp` and `AppConfig.llama_cpp = Some(LlamaCppConfig { ... })`. The existing TOML write path serializes the full struct — no special handling needed if serde is configured correctly.

### 0.5.4 Ollama Path Unchanged

When `Ollama` is selected (or defaulted), the flow is identical to today. The `llama_cpp` field remains `None`.

---

## 0.6 — Convenience Methods

Add to `impl AppConfig`:

```rust
pub fn is_llamacpp(&self) -> bool {
    self.llm_backend == LlmBackend::LlamaCpp
}

pub fn is_ollama(&self) -> bool {
    self.llm_backend == LlmBackend::Ollama
}
```

These avoid `match` boilerplate in the 20+ call sites that will branch on backend.

---

## 0.7 — Tests

All tests in `src/config.rs` `#[cfg(test)]` module:

| # | Test name | What it validates |
|---|-----------|-------------------|
| 1 | `round_trip_llamacpp_config` | Serialize `AppConfig` with `LlmBackend::LlamaCpp` + full `LlamaCppConfig` to TOML, deserialize back, assert equality |
| 2 | `missing_backend_defaults_to_ollama` | Deserialize a TOML string with no `llm_backend` key → `LlmBackend::Ollama` |
| 3 | `missing_llamacpp_section_is_none` | Deserialize TOML with `llm_backend = "Ollama"` and no `[llama_cpp]` → `llama_cpp` is `None` |
| 4 | `validate_llamacpp_catches_missing_section` | `llm_backend = LlamaCpp`, `llama_cpp = None` → `FcpError::Config` |
| 5 | `validate_llamacpp_catches_missing_binary` | Point `home` to a `tempdir` without `bin/llama-server` → `FcpError::Config` |
| 6 | `validate_llamacpp_catches_missing_gguf` | Valid `home` with binary, but `chat_model_path` points to nonexistent file → `FcpError::Config` |
| 7 | `is_llamacpp_and_is_ollama_helpers` | Verify the convenience predicates |
| 8 | `default_urls_populated` | Deserialize with no `chat_server_url` → defaults to `http://127.0.0.1:8090` |

**All file-system tests use `tempfile::TempDir`** per workspace rule.

---

## 0.8 — Files Summary

| File | Action | What changes |
|------|--------|-------------|
| `src/config.rs` | Modify | Add `LlmBackend`, `LlamaCppConfig`, two `AppConfig` fields, validation method, helpers |
| `src/executive/error.rs` | No change | `FcpError::Config(String)` already exists |
| `src/executive/ignition.rs` | Modify | Backend selector + llama.cpp sub-prompts |

---

## 0.9 — Acceptance Criteria

- [ ] `cargo build` passes with zero warnings
- [ ] `cargo test` passes, including the 8 new tests
- [ ] Existing vault `config.toml` files (no `llm_backend`) load without error
- [ ] `eris chat` in a fresh directory (no seal file) triggers ignition with the new backend selector
- [ ] Selecting Ollama produces identical behavior to pre-refactor
- [ ] Selecting llama.cpp writes a valid `[llama_cpp]` section to `config.toml`
- [ ] No runtime behavior changes — engine instantiation is still Ollama-only (Phase 1 wires it up)
- [ ] Entry point is always `eris chat` — no new CLI subcommand introduced
