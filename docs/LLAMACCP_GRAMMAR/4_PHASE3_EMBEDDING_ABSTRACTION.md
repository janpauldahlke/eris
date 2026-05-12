# Phase 3 — Embedding Abstraction

**Depends on:** Phase 0 (config/backend enum), Phase 2 (embed server running)
**Unlocks:** Phase 6 (health checks for both backends)
**Estimated scope:** ~150 LOC new, ~80 LOC modified, ~6 tests

---

## 3.1 — Goal

`ToolRouter` and `SemanticBrain` currently take `Arc<Ollama>` directly and call `ollama.generate_embeddings(...)`. Extract an `EmbeddingProvider` trait so both backends can supply vectors without the consumers knowing which backend is active.

---

## 3.2 — Current Coupling Points

### 3.2.1 `ToolRouter` (`src/orchestrator/tool_router.rs`)

```rust
pub struct ToolRouter {
    ollama: Arc<Ollama>,
    embed_model: String,
    tool_embeddings: Vec<(String, Vec<f32>)>,
    threshold: f32,
}
```

Uses `Self::embed(&self.ollama, &self.embed_model, &text)` which calls `ollama.generate_embeddings(request)`.

### 3.2.2 `SemanticBrain` (`src/memory/semantic.rs`)

```rust
pub struct SemanticBrain {
    client: Arc<Qdrant>,
    ollama: Arc<Ollama>,
    config: Arc<AppConfig>,
}
```

Calls `self.ollama.generate_embeddings(request)` with `self.config.embed_model_name`.

Both follow the same pattern: build a `GenerateEmbeddingsRequest`, call `generate_embeddings`, extract the first vector.

---

## 3.3 — New File: `src/engine/embedding.rs`

### 3.3.1 Trait Definition

```rust
use crate::executive::error::Result;
use async_trait::async_trait;

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Generate an embedding vector for a single text input.
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;

    /// Embedding vector dimensionality (used for Qdrant collection validation at startup).
    fn dimensions(&self) -> usize;
}
```

The `dimensions()` method is critical: Qdrant collections have a fixed vector width. When switching backends, mismatched dimensions cause silent failures. Phase 6 validates this at startup.

### 3.3.2 `OllamaEmbedding` Implementation

```rust
use ollama_rs::Ollama;
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;

pub struct OllamaEmbedding {
    ollama: Arc<Ollama>,
    model: String,
    dimensions: usize,
}

impl OllamaEmbedding {
    pub fn new(ollama: Arc<Ollama>, model: String) -> Self {
        // nomic-embed-text produces 768-dimensional vectors
        Self { ollama, model, dimensions: 768 }
    }
}

#[async_trait]
impl EmbeddingProvider for OllamaEmbedding {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let request = GenerateEmbeddingsRequest::new(
            self.model.clone(),
            text.to_string().into(),
        );
        let response = self.ollama
            .generate_embeddings(request)
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;
        response.embeddings.into_iter().next().ok_or_else(|| {
            FcpError::NetworkFault("Ollama returned empty embeddings".into())
        })
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }
}
```

**Note on dimensions:** 768 is the default for `nomic-embed-text`. Ideally this should be determined dynamically (first call + cache), but hardcoding is acceptable since the model is operator-chosen and the dimension is well-known. If a future model differs, the operator sets a config override.

### 3.3.3 `LlamaCppEmbedding` Implementation

```rust
pub struct LlamaCppEmbedding {
    http: reqwest::Client,
    embed_url: String,    // "http://127.0.0.1:8091/v1/embeddings"
    dimensions: usize,
}

impl LlamaCppEmbedding {
    pub fn new(embed_server_url: &str, timeout_secs: u64) -> Result<Self> {
        let embed_url = format!(
            "{}/v1/embeddings",
            embed_server_url.trim_end_matches('/')
        );
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .map_err(|e| FcpError::NetworkFault(format!("embed HTTP client: {e}")))?;
        // nomic-embed-text GGUF also produces 768 dims; validated at startup in Phase 6
        Ok(Self { http, embed_url, dimensions: 768 })
    }
}

#[async_trait]
impl EmbeddingProvider for LlamaCppEmbedding {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let body = serde_json::json!({
            "input": text,
        });
        let resp = self.http.post(&self.embed_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| FcpError::NetworkFault(format!("embed request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default(); // unwrap_or_default on Result<String> is fine (not Option panic)
            return Err(FcpError::NetworkFault(
                format!("embed server returned {status}: {body}")
            ));
        }

        let parsed: EmbeddingResponse = resp.json().await
            .map_err(|e| FcpError::NetworkFault(format!("embed response parse: {e}")))?;

        parsed.data.into_iter().next()
            .map(|d| d.embedding)
            .ok_or_else(|| FcpError::NetworkFault("embed server returned empty data".into()))
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}
```

**llama-server `/v1/embeddings` API contract:**
- Request: `{ "input": "text" }` — no model field needed (the model is set at server startup via `--model`)
- Response: `{ "data": [{ "embedding": [0.1, 0.2, ...] }] }`
- The server must have been started with `--embedding` flag

---

## 3.4 — Module Registration: `src/engine/mod.rs`

```rust
pub mod embedding;
```

Re-export:
```rust
pub use self::embedding::EmbeddingProvider;
```

---

## 3.5 — Refactor `ToolRouter` (`src/orchestrator/tool_router.rs`)

### 3.5.1 Field Changes

Replace:
```rust
ollama: Arc<Ollama>,
embed_model: String,
```

With:
```rust
embed: Arc<dyn EmbeddingProvider>,
```

### 3.5.2 Constructor Change

```rust
pub async fn new(
    embed: Arc<dyn EmbeddingProvider>,   // was: ollama + embed_model
    tool_descriptions: Vec<(String, String)>,
    descriptors: Option<Arc<ToolDescriptorRegistry>>,
    threshold: f32,
) -> Result<Self>
```

### 3.5.3 `embed` Static Method → Instance Method

The current static `Self::embed(&ollama, &embed_model, &text)` becomes:

```rust
async fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
    self.embed.embed(text).await
}
```

All call sites within `ToolRouter` update from `Self::embed(&self.ollama, &self.embed_model, text)` to `self.embed_text(text)`.

---

## 3.6 — Refactor `SemanticBrain` (`src/memory/semantic.rs`)

### 3.6.1 Field Changes

Replace:
```rust
ollama: Arc<Ollama>,
```

With:
```rust
embed: Arc<dyn EmbeddingProvider>,
```

### 3.6.2 Constructor Change

Both `new` and `new_with_connect_retries` need updating (the latter is the one actually used in `chat_session.rs`):

```rust
pub async fn new(
    config: Arc<AppConfig>,
    embed: Arc<dyn EmbeddingProvider>,   // was: ollama: Arc<Ollama>
) -> Result<Self>

pub async fn new_with_connect_retries(
    config: Arc<AppConfig>,
    embed: Arc<dyn EmbeddingProvider>,   // was: ollama: Arc<Ollama>
    max_attempts: u32,
    retry_delay_ms: u64,
) -> Result<Self>
```

### 3.6.3 Internal Embed Calls

Replace all `self.ollama.generate_embeddings(...)` calls with `self.embed.embed(text).await?`.

Remove the `use ollama_rs::...` imports from `semantic.rs`.

---

## 3.7 — Wiring in `chat_session.rs`

### 3.7.1 Construct the Right Provider

```rust
let embed_provider: Arc<dyn EmbeddingProvider> = match config.llm_backend {
    LlmBackend::Ollama => {
        Arc::new(OllamaEmbedding::new(
            ollama.clone(),
            config.embed_model_name.clone(),
        ))
    }
    LlmBackend::LlamaCpp => {
        let lc = config.validate_llamacpp_config()?;
        Arc::new(LlamaCppEmbedding::new(
            &lc.embed_server_url,
            config.generation_timeout_secs,
        )?)
    }
};
```

### 3.7.2 Inject into Consumers

```rust
let tool_router = ToolRouter::new(
    embed_provider.clone(),
    tool_descriptions,
    descriptors,
    config.tool_match_threshold,
).await?;

// Note: chat_session.rs uses new_with_connect_retries, not new
let semantic_brain = SemanticBrain::new_with_connect_retries(
    config.clone(),
    embed_provider.clone(),
    config.semantic_brain_connect_attempts,
    config.semantic_brain_connect_retry_delay_ms,
).await?;
```

---

## 3.8 — Dimension Validation Consideration

When switching from Ollama's `nomic-embed-text` (768 dims) to a llama.cpp GGUF model, the vector dimensions **must match** the existing Qdrant collection. If they don't, upserts and queries will fail silently or with cryptic Qdrant errors.

**Phase 6** adds a startup validation that checks `embed_provider.dimensions()` against the Qdrant collection's configured vector size. For Phase 3, just ensure `dimensions()` returns the correct value.

---

## 3.9 — Tests

| # | Test name | Location | What it validates |
|---|-----------|----------|-------------------|
| 1 | `ollama_embedding_delegates_correctly` | `embedding.rs` | Mock `Ollama` → `OllamaEmbedding::embed` returns expected vector |
| 2 | `llamacpp_embedding_valid_response` | `embedding.rs` | Wiremock `/v1/embeddings` → `LlamaCppEmbedding::embed` returns vector |
| 3 | `llamacpp_embedding_server_error` | `embedding.rs` | Mock 500 → `NetworkFault` |
| 4 | `llamacpp_embedding_empty_data` | `embedding.rs` | Mock returns empty `data` array → `NetworkFault` |
| 5 | `tool_router_uses_provider` | `tool_router.rs` | Construct with a mock `EmbeddingProvider`, verify routing works |
| 6 | `dimensions_returns_expected` | `embedding.rs` | Both impls return 768 |

For test 5, create a test-only `MockEmbeddingProvider` that returns a fixed vector. This validates that `ToolRouter` doesn't depend on Ollama-specific types anymore.

---

## 3.10 — Files Summary

| File | Action | What changes |
|------|--------|-------------|
| `src/engine/embedding.rs` | Create | `EmbeddingProvider` trait, `OllamaEmbedding`, `LlamaCppEmbedding` |
| `src/engine/mod.rs` | Modify | Add `pub mod embedding;` and re-export |
| `src/orchestrator/tool_router.rs` | Modify | Replace `Arc<Ollama>` + `embed_model` with `Arc<dyn EmbeddingProvider>` |
| `src/memory/semantic.rs` | Modify | Replace `Arc<Ollama>` with `Arc<dyn EmbeddingProvider>` |
| `src/executive/chat_session.rs` | Modify | Construct the right provider, inject into ToolRouter and SemanticBrain |

---

## 3.11 — Acceptance Criteria

- [ ] `ToolRouter` and `SemanticBrain` no longer import `ollama_rs` embedding types
- [ ] Both work with `OllamaEmbedding` (regression: existing Ollama path)
- [ ] Both work with `LlamaCppEmbedding` + running embed server
- [ ] Semantic memory (memory:query, memory:commit) works on both backends
- [ ] Tool routing (pre-LLM semantic match) works on both backends
- [ ] No `Arc<Ollama>` leaks into ToolRouter or SemanticBrain
- [ ] `cargo build` passes, `cargo test` passes
