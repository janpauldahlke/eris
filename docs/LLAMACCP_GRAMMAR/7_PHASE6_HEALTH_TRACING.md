# Phase 6 — Health Check and Tracing Integration

**Depends on:** Phase 1 (LlamaCppClient publishes token metrics), Phase 2 (llama-server processes), Phase 3 (embedding provider)
**Unlocks:** Nothing (leaf phase, can run in parallel with Phase 7/8)
**Estimated scope:** ~100 LOC new, ~60 LOC modified, ~6 tests

---

## 6.1 — Goal

Make `system:health` report the active backend. Ensure tracing covers the new engine. Validate token metrics work for both paths. Add preflight checks for llama-server reachability. Validate embedding vector dimensions at startup.

---

## 6.2 — `system:health` Changes (`src/tools/system/health.rs`)

### 6.2.1 Add `llm_backend` to Health Output

The health JSON currently has an `fcp` section with Ollama details. Extend it:

```json
{
  "report_hint": "...",
  "llm_backend": "Ollama",
  "fcp": {
    "ollama_host": "http://localhost:11434",
    "chat_model": "qwen2.5:14b",
    "embed_model": "nomic-embed-text"
  },
  "cpu": { ... },
  "memory": { ... }
}
```

When llama.cpp is active:

```json
{
  "report_hint": "...",
  "llm_backend": "LlamaCpp",
  "fcp": {
    "chat_server": "http://127.0.0.1:8090",
    "chat_model": "/models/qwen2.5-14b.gguf",
    "embed_server": "http://127.0.0.1:8091",
    "embed_model": "/models/nomic-embed-text.gguf"
  },
  "llama_cpp_health": {
    "chat_server_status": "ok",
    "embed_server_status": "ok"
  },
  "cpu": { ... },
  "memory": { ... }
}
```

### 6.2.2 Implementation

In the `execute` method, branch on `self.config.llm_backend`:

```rust
let fcp_section = match self.config.llm_backend {
    LlmBackend::Ollama => {
        json!({
            "ollama_host": self.config.ollama_host,
            "chat_model": self.config.model_name,
            "embed_model": self.config.embed_model_name,
        })
    }
    LlmBackend::LlamaCpp => {
        let lc = self.config.llama_cpp.as_ref();
        json!({
            "chat_server": lc.map(|c| &c.chat_server_url),
            "chat_model": lc.map(|c| c.chat_model_path.display().to_string()),
            "embed_server": lc.map(|c| &c.embed_server_url),
            "embed_model": lc.map(|c| c.embed_model_path.display().to_string()),
        })
    }
};
```

### 6.2.3 llama-server Health Probes

When llama.cpp is active, query both servers' `/health` endpoints:

```rust
let llama_health = if self.config.is_llamacpp() {
    let lc = self.config.llama_cpp.as_ref().unwrap(); // safe: validated at startup
    let chat_status = probe_llama_health(&lc.chat_server_url).await;
    let embed_status = probe_llama_health(&lc.embed_server_url).await;
    Some(json!({
        "chat_server_status": chat_status,
        "embed_server_status": embed_status,
    }))
} else {
    None
};
```

`probe_llama_health` is a simple GET to `{url}/health`, returns `"ok"`, `"loading model"`, or `"unreachable: {error}"`.

### 6.2.4 Update `report_hint`

The `REPORT_HINT` constant needs a backend-aware version:

```rust
const REPORT_HINT_LLAMACPP: &str = "When answering the user, cover: (1) `llm_backend` and `fcp` section (chat/embed servers and models); (2) `llama_cpp_health` server statuses; (3) `cpu.usage_pct` and load averages; (4) `memory` used vs total. If GPU info is available, include it.";
```

Use the appropriate hint based on backend.

### 6.2.5 Skip `ollama ps` When llama.cpp Active

The current health tool runs `ollama ps` as a subprocess. Skip this when llama.cpp is active:

```rust
let ollama_ps = if self.config.is_ollama() {
    // existing ollama ps logic
} else {
    json!({"skipped": "llama.cpp backend active"})
};
```

---

## 6.3 — Preflight Changes (`src/telemetry/preflight.rs`)

### 6.3.1 Backend-Aware Reachability

Currently, preflight checks Ollama and Qdrant for non-Chat commands. Add llama-server checks:

```rust
pub async fn run_preflight_checks(command: &Commands, config: &AppConfig) -> Result<()> {
    if matches!(command, Commands::Chat { .. } | Commands::Benchmark { .. }) {
        return Ok(());
    }

    match config.llm_backend {
        LlmBackend::Ollama => {
            if !ollama_reachable(&config.ollama_host).await {
                return Err(FcpError::NetworkFault(
                    "FATAL: Ollama daemon not responding.".into(),
                ));
            }
        }
        LlmBackend::LlamaCpp => {
            if let Some(lc) = &config.llama_cpp {
                if !llama_server_reachable(&lc.chat_server_url).await {
                    return Err(FcpError::NetworkFault(
                        format!("FATAL: llama-server (chat) not responding at {}", lc.chat_server_url),
                    ));
                }
                if !llama_server_reachable(&lc.embed_server_url).await {
                    return Err(FcpError::NetworkFault(
                        format!("FATAL: llama-server (embed) not responding at {}", lc.embed_server_url),
                    ));
                }
            }
        }
    }

    if !qdrant_reachable(&config.qdrant_url).await {
        return Err(FcpError::NetworkFault(
            "FATAL: Qdrant sidecar not detected.".into(),
        ));
    }
    Ok(())
}
```

### 6.3.2 `llama_server_reachable` in `peripherals.rs`

Add a public reachability function (similar to `ollama_reachable`):

```rust
pub async fn llama_server_reachable(url: &str) -> bool {
    let health_url = format!("{}/health", url.trim_end_matches('/'));
    match reqwest::get(&health_url).await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}
```

---

## 6.4 — Embedding Dimension Validation

### 6.4.1 Startup Check

In `chat_session.rs`, after constructing the `EmbeddingProvider` and before constructing `SemanticBrain`:

```rust
// Validate embedding dimensions match Qdrant collection
let expected_dims = embed_provider.dimensions();
let collection_dims = semantic_brain_collection_dimensions(&config).await?;
if let Some(coll_dims) = collection_dims {
    if coll_dims != expected_dims {
        return Err(FcpError::Config(format!(
            "Embedding dimension mismatch: provider produces {expected_dims}-dim vectors, \
             but Qdrant collection '{}' expects {coll_dims}-dim. \
             Either use a compatible embedding model or recreate the collection.",
            config.qdrant_collection_v2
        )));
    }
}
```

### 6.4.2 `semantic_brain_collection_dimensions`

Add to `src/memory/semantic.rs`:

```rust
pub async fn collection_dimensions(config: &AppConfig) -> Result<Option<usize>> {
    let client = Qdrant::from_url(&config.qdrant_url).build()
        .map_err(|e| FcpError::NetworkFault(e.to_string()))?;
    match client.collection_info(&config.qdrant_collection_v2).await {
        Ok(info) => {
            // Extract vector dimension from collection config
            // Qdrant API: info.config.params.vectors.size
            Ok(Some(info.config.params.vectors.size as usize))
        }
        Err(_) => Ok(None), // Collection doesn't exist yet; SemanticBrain::new creates it
    }
}
```

The exact Qdrant client API for getting collection dimensions needs verification against the `qdrant_client` crate version in `Cargo.toml`. The concept is: if the collection already exists, check its vector size matches. If it doesn't exist yet, skip the check (it will be created with the right dimensions).

---

## 6.5 — Token Metrics Verification

Phase 1 already has `LlamaCppClient` publishing to `token_metrics_tx`. This phase adds **integration tests** to verify the pipeline works end-to-end:

1. Construct `LlamaCppClient` with a `watch::channel` receiver
2. Mock a response with `usage` fields
3. Verify the `LlmTokenSnapshot` is published with correct values
4. Verify `TokenMetricsReader` reads the snapshot

This is mostly a test exercise — the code was written in Phase 1.

---

## 6.6 — Tracing Field Consistency

Audit all `tracing::info!` / `tracing::debug!` events in `LlamaCppClient` and `LlamaCppEmbedding` to use consistent field names with the Ollama counterparts:

| Field | Ollama value | LlamaCpp value |
|-------|-------------|----------------|
| `engine` | `"ollama"` | `"llamacpp"` |
| `model` | model name | GGUF filename |
| `prompt_tokens` | from response | from response |
| `completion_tokens` | from response | from response |
| `generation_ms` | from timing | from timing |

Add `engine = "llamacpp"` to key tracing events in `LlamaCppClient::generate` so log analysis can filter by backend.

---

## 6.7 — Tests

| # | Test name | Location | What it validates |
|---|-----------|----------|-------------------|
| 1 | `health_output_ollama_backend` | `health.rs` | Ollama backend → output contains `ollama_host`, no `llama_cpp_health` |
| 2 | `health_output_llamacpp_backend` | `health.rs` | LlamaCpp backend → output contains `chat_server`, `llama_cpp_health` |
| 3 | `preflight_llamacpp_unreachable` | `preflight.rs` | LlamaCpp with unreachable URL → `NetworkFault` |
| 4 | `preflight_ollama_skipped_for_llamacpp` | `preflight.rs` | LlamaCpp backend doesn't check Ollama reachability |
| 5 | `token_metrics_publish_llamacpp` | `llama_cpp.rs` or `token_metrics.rs` | Wiremock response → snapshot published with correct counts |
| 6 | `dimension_mismatch_fails_fast` | `chat_session.rs` or `semantic.rs` | Provider says 768, collection says 384 → `FcpError::Config` |

---

## 6.8 — Files Summary

| File | Action | What changes |
|------|--------|-------------|
| `src/tools/system/health.rs` | Modify | Add `llm_backend` field, llama-server health probes, skip `ollama ps` for llama.cpp |
| `src/telemetry/preflight.rs` | Modify | Backend-aware reachability checks |
| `src/executive/peripherals.rs` | Modify | Add `llama_server_reachable()` |
| `src/memory/semantic.rs` | Modify | Add `collection_dimensions()` for startup validation |
| `src/executive/chat_session.rs` | Modify | Dimension validation at startup |
| `src/engine/llama_cpp.rs` | Modify | Consistent tracing fields |

---

## 6.9 — Acceptance Criteria

- [ ] `system:health` reports correct backend and server details for both paths
- [ ] `system:health` with llama.cpp shows live health status of both servers
- [ ] `system:health` with Ollama is unchanged from pre-refactor
- [ ] Preflight checks work correctly for both backends
- [ ] Embedding dimension mismatch is caught at startup with clear error
- [ ] Token metrics publish correctly for llama.cpp (verified by integration test)
- [ ] Tracing events use consistent field names across backends
