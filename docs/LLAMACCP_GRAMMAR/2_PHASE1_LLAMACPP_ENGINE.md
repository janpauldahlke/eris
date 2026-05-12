# Phase 1 — LlamaCppClient: Engine Implementation

**Depends on:** Phase 0 (config types, `LlmBackend` enum)
**Unlocks:** Phase 2 (process mgmt), Phase 4 (grammar), Phase 5 (recovery)
**Estimated scope:** ~350 LOC new, ~40 LOC modified, ~10 tests

---

## 1.1 — Goal

Implement `LlmEngine` for llama-server's OpenAI-compatible `/v1/chat/completions` endpoint. **No grammar yet** — this phase validates the HTTP integration, streaming, and token metrics before grammar complexity is layered on.

After this phase, `eris chat` works end-to-end with a running `llama-server` if you manually start it.

---

## 1.2 — New File: `src/engine/llama_cpp.rs`

### 1.2.1 Struct Definition

```rust
pub struct LlamaCppClient {
    http: reqwest::Client,
    chat_url: String,           // "http://127.0.0.1:8090/v1/chat/completions"
    config: Arc<AppConfig>,
    token_metrics_tx: Option<tokio::sync::watch::Sender<LlmTokenSnapshot>>,
}
```

### 1.2.2 Constructor

```rust
impl LlamaCppClient {
    pub fn new(config: Arc<AppConfig>) -> Result<Self> {
        let lc = config.validate_llamacpp_config()?;
        let chat_url = format!("{}/v1/chat/completions", lc.chat_server_url.trim_end_matches('/'));
        let timeout = Duration::from_secs(config.generation_timeout_secs);
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| FcpError::NetworkFault(format!("HTTP client build: {e}")))?;
        Ok(Self {
            http,
            chat_url,
            config,
            token_metrics_tx: None,
        })
    }

    pub fn with_token_metrics(mut self, tx: tokio::sync::watch::Sender<LlmTokenSnapshot>) -> Self {
        self.token_metrics_tx = Some(tx);
        self
    }
}
```

### 1.2.3 `impl LlmEngine` — `generate` method

The method must:

1. **Build the messages array** from `&[Message]` in OpenAI format:
   ```json
   [
     {"role": "system", "content": "..."},
     {"role": "user", "content": "..."},
     {"role": "assistant", "content": "..."}
   ]
   ```
   Same role mapping as `OllamaClient` — iterate the `Message` slice, map `role` and `content` directly.

2. **Build the request body:**
   ```json
   {
     "messages": [...],
     "stream": true,
     "temperature": 0.7,
     "n_predict": -1
   }
   ```
   - `stream: true` for the streaming path (when `stream_tx` is `Some`)
   - `stream: false` for the non-streaming path
   - `temperature` from config if we add one, or hardcode `0.7` matching Ollama defaults
   - `n_predict: -1` means unlimited (let context window be the limit)
   - **Important:** Do NOT set `grammar` here — that's Phase 4. Leave a `// Phase 4: grammar field here` marker.

3. **POST to `self.chat_url`**

4. **Parse the response:**

   **Non-streaming path (`stream_tx` is `None`):**
   - Read the full response body as JSON
   - Extract `choices[0].message.content` as the `content` string
   - Extract `usage.prompt_tokens` and `usage.completion_tokens`

   **Streaming path (`stream_tx` is `Some`):**
   - Read SSE lines (`data: {...}`)
   - For each chunk: extract `choices[0].delta.content` and send to `stream_tx`
   - Handle `data: [DONE]` sentinel
   - The final chunk before `[DONE]` contains `usage` with token counts
   - Accumulate all content deltas into the final `EngineResponse.content`

5. **Publish token metrics** via `self.token_metrics_tx` (same pattern as `OllamaClient`)

6. **Error handling:**
   - HTTP non-2xx → `FcpError::NetworkFault` with status code and body excerpt
   - Timeout → `FcpError::NetworkFault("llama-server request timed out")`
   - JSON parse failure on response → `FcpError::ParseFault`
   - Connection refused → `FcpError::NetworkFault` with clear message suggesting llama-server isn't running

### 1.2.4 SSE Parsing

llama-server's streaming format:
```
data: {"id":"...","choices":[{"delta":{"content":"Hello"}}],...}

data: {"id":"...","choices":[{"delta":{"content":" world"}}],"usage":{"prompt_tokens":42,"completion_tokens":5}}

data: [DONE]
```

Key considerations:
- Lines are newline-delimited, prefixed with `data: `
- Empty lines between events (skip them)
- `content` can be `null` or absent in some chunks (e.g., the role-only first chunk)
- `usage` appears in the last non-DONE chunk when llama-server is configured with `--metrics`

Implement a helper function:

```rust
async fn stream_sse_response(
    response: reqwest::Response,
    stream_tx: &mpsc::UnboundedSender<String>,
) -> Result<(String, usize, usize)>
```

Returns `(full_content, prompt_tokens, completion_tokens)`.

### 1.2.5 `available_tools_json` Parameter

The `generate` signature includes `available_tools_json: &str`. In this phase, it is **ignored** — tool schemas are embedded in the system prompt by the `ContextAssembler`, same as the current Ollama path. The grammar phases (4/7) will use this parameter to build dynamic GBNF.

---

## 1.3 — Module Registration: `src/engine/mod.rs`

Add:
```rust
pub mod llama_cpp;
```

And the re-export:
```rust
pub use self::llama_cpp::LlamaCppClient;
```

---

## 1.4 — Engine Instantiation: `src/executive/chat_session.rs`

Locate the point where `OllamaClient` is created and injected into the `Orchestrator`. Add a branch:

```rust
let engine: Box<dyn LlmEngine> = match config.llm_backend {
    LlmBackend::Ollama => {
        // existing OllamaClient construction
        Box::new(ollama_client)
    }
    LlmBackend::LlamaCpp => {
        let client = LlamaCppClient::new(config.clone())?
            .with_token_metrics(token_tx);
        Box::new(client)
    }
};
```

**Critical:** The `Orchestrator` is currently generic over `E: LlmEngine` (see `Orchestrator<E>`). Check whether it uses `E` as a type parameter or as a trait object. Based on the step.rs signature `impl<E: LlmEngine> Orchestrator<E>`, it's **generic**. This means we either:

- **Option A:** Make `Orchestrator` use `Box<dyn LlmEngine>` instead of `E: LlmEngine` (simplifies runtime dispatch but touches many files)
- **Option B:** Keep the generic and use an enum-dispatch pattern

**Recommended: Option A** — Change `Orchestrator<E: LlmEngine>` to `Orchestrator` with a `engine: Box<dyn LlmEngine>` field. This is a significant but mechanical refactor:

1. Remove the `<E: LlmEngine>` type parameter from `Orchestrator`
2. Change the `engine` field to `Box<dyn LlmEngine>`
3. Remove `<E>` from all `impl<E: LlmEngine> Orchestrator<E>` blocks (there are likely 5-10)
4. Remove `<E>` from all construction sites

**Alternative if Option A is too disruptive for Phase 1:** Use an enum wrapper:

```rust
pub enum AnyEngine {
    Ollama(OllamaClient),
    LlamaCpp(LlamaCppClient),
}

#[async_trait]
impl LlmEngine for AnyEngine {
    async fn generate(&self, stack: &[Message], tools_json: &str, tx: Option<...>) -> Result<EngineResponse> {
        match self {
            Self::Ollama(e) => e.generate(stack, tools_json, tx).await,
            Self::LlamaCpp(e) => e.generate(stack, tools_json, tx).await,
        }
    }
}
```

Then `Orchestrator<AnyEngine>` compiles without changing the generic. Less invasive, but adds a layer. Pick whichever fits the codebase better after inspecting how many `impl<E>` blocks exist.

---

## 1.5 — Request/Response Types (Internal to `llama_cpp.rs`)

Define private serde structs for the OpenAI-compatible API:

```rust
#[derive(Serialize)]
struct ChatCompletionRequest {
    messages: Vec<ChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    n_predict: Option<i32>,
    // Phase 4: grammar field
    #[serde(skip_serializing_if = "Option::is_none")]
    grammar: Option<String>,
}

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Choice {
    message: Option<MessageContent>,
    delta: Option<DeltaContent>,
}

#[derive(Deserialize)]
struct MessageContent { content: String }

#[derive(Deserialize)]
struct DeltaContent { content: Option<String> }

#[derive(Deserialize)]
struct Usage {
    prompt_tokens: usize,
    completion_tokens: usize,
}
```

---

## 1.6 — Tests

### 1.6.1 Unit Tests (wiremock)

Use `wiremock` (already a dev-dependency from Ollama tests) to mock `llama-server`:

| # | Test name | What it validates |
|---|-----------|-------------------|
| 1 | `non_streaming_valid_response` | POST to mock, get back a full response, verify `EngineResponse` fields |
| 2 | `streaming_valid_response` | Mock returns SSE chunks, verify content accumulation and token counts |
| 3 | `streaming_forwards_deltas_to_tx` | Verify each content delta is sent through `stream_tx` |
| 4 | `http_timeout_returns_network_fault` | Mock with delay beyond timeout → `FcpError::NetworkFault` |
| 5 | `http_500_returns_network_fault` | Mock returns 500 → descriptive error |
| 6 | `connection_refused_returns_network_fault` | Use unreachable port → error message mentions llama-server |
| 7 | `missing_usage_defaults_to_zero` | Response without `usage` → tokens default to 0 |
| 8 | `empty_content_in_delta_skipped` | SSE chunk with `"content": null` doesn't crash or inject "null" |
| 9 | `done_sentinel_terminates_stream` | Verify `data: [DONE]` cleanly ends the loop |
| 10 | `constructor_validates_config` | `LlamaCppClient::new` with valid config succeeds, with broken config fails |

### 1.6.2 Integration Smoke Test (manual, not CI)

Documented in the test file as `#[ignore]`:
- Start `llama-server` manually with a small model
- Run `cargo test -- --ignored llamacpp_smoke`
- Verify conversational response

---

## 1.7 — Files Summary

| File | Action | What changes |
|------|--------|-------------|
| `src/engine/llama_cpp.rs` | Create | Full `LlamaCppClient` implementation |
| `src/engine/mod.rs` | Modify | Add `pub mod llama_cpp;` and re-export |
| `src/executive/chat_session.rs` | Modify | Branch on `LlmBackend` for engine construction |
| `src/engine/router.rs` (maybe) | Modify | If `EngineRouter` wraps the engine, add the branch there |

---

## 1.8 — Acceptance Criteria

- [ ] `cargo build` passes
- [ ] All wiremock tests pass
- [ ] `eris chat` with `llm_backend = "Ollama"` works identically to before
- [ ] `eris chat` with `llm_backend = "LlamaCpp"` + manually running `llama-server` produces conversational responses
- [ ] Token metrics publish correctly for both backends
- [ ] Streaming works (TUI shows incremental text)
- [ ] Timeout and connection errors produce clear `FcpError` messages
- [ ] No `unwrap()` or `expect()` in production code
