# Engine: LLM, embeddings, grammar, and auxiliary routing

## LlmEngine trait (`engine/traits.rs`)

`LlmEngine::generate(stack, available_tools_json, stream_tx)`:

- **`stack`:** `&[Message]` with roles `system` | `user` | `assistant`.
- **`available_tools_json`:** second argument; not used by the hot path in either backend (tool schemas live inside the assembled system prompt). `LlamaCppClient` uses its cached grammar instead.
- **`stream_tx`:** optional; TUI can stream tokens (not all paths enable it).

Two implementations compile unconditionally; `AppConfig::llm_backend` determines which is instantiated at runtime.

## Ollama client (`engine/ollama.rs`)

- Builds `ChatMessageRequest` with `FormatType::Json` for structured assistant output.
- `GenerationOptions::num_ctx` from `AppConfig`.
- Timeouts on connect and stream chunks.
- On success, publishes `LlmTokenSnapshot` via `token_metrics` watch channel for the UI header.
- No grammar enforcement — model output is parsed and validated post-hoc.

## LlamaCpp client (`engine/llama_cpp.rs`)

- POSTs to llama-server's **`/v1/chat/completions`** (OpenAI-compatible endpoint).
- Builds an OpenAI-compatible `messages` array from the `&[Message]` stack (same role mapping as `OllamaClient`).
- Attaches a **GBNF grammar** string (compiled at session start, cached as `Option<Arc<String>>`) to every request. This constrains output to valid FCP protocol JSON — parse failures are structurally impossible.
- Supports SSE streaming (`stream: true`): parses `data:` lines, forwards content deltas to `stream_tx`.
- Extracts `usage.prompt_tokens` / `usage.completion_tokens` from the final chunk.
- Publishes to `token_metrics_tx` (same UI header metrics as Ollama).

```rust
pub struct LlamaCppClient {
    http: reqwest::Client,
    chat_url: String,
    config: Arc<AppConfig>,
    token_metrics_tx: Option<watch::Sender<LlmTokenSnapshot>>,
    grammar: Option<Arc<String>>,
}
```

Grammar is set via `set_grammar()` after construction, called from `chat_session.rs` once the dynamic grammar is compiled from registered tools.

## EmbeddingProvider trait (`engine/embedding.rs`)

Abstracts vector generation so `ToolRouter` and `SemanticBrain` are backend-agnostic:

```rust
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
    fn dimensions(&self) -> usize;
}
```

Two implementations:

- **`OllamaEmbedding`** — wraps `Arc<Ollama>` + `embed_model_name`; extracts the logic that was previously inlined in `ToolRouter` and `SemanticBrain`.
- **`LlamaCppEmbedding`** — wraps `reqwest::Client` + embed server URL; calls llama-server's `/v1/embeddings` endpoint.

`ToolRouter` and `SemanticBrain` both take `Arc<dyn EmbeddingProvider>` at construction. `chat_session.rs` instantiates the right provider based on `llm_backend`.

## GBNF grammar compiler (`engine/grammar/`)

Compiles a grammar that constrains llama-server output to valid FCP protocol JSON. Three submodules:

### `envelope.rs` — static protocol skeleton

`compile_fcp_envelope_grammar(tool_names)` builds a GBNF string enforcing:

- Root is a single JSON object
- Required keys: `thought` (string), `status` (enum: `"Task"`, `"Reflect"`, `"Idle"`, `"Process"`), `tool_calls` (array)
- Optional key: `message_to_user` (string or null)
- Each `tool_calls` element: `{ "name": <tool_name_enum>, "args": <json_object> }`

`compile_fcp_envelope_grammar_dynamic(tool_names, tool_schemas)` extends this with **per-tool argument schemas** (dynamic grammar).

### `tool_names.rs` — dynamic tool name enum

Builds the GBNF rule for valid tool names from `Gatekeeper::registered_tool_names()`:

```
tool-name ::= "\"vault:read\"" | "\"vault:write\"" | "\"memory:stage\"" | ...
```

### `schema_to_gbnf.rs` — JSON Schema to GBNF compiler

Translates each tool's `parameters_schema()` (`schemars::RootSchema`) into GBNF rules. Handles: object, string, number, integer, boolean, enum, array, required/optional fields, `$ref` resolution. If a schema construct is unsupported, falls back to freeform JSON object for that tool's args (graceful degradation).

**Session lifecycle:**
1. At session start, `chat_session.rs` collects registered tool names and schemas from the gatekeeper.
2. `compile_fcp_envelope_grammar_dynamic()` produces the complete GBNF string.
3. The string is passed to `LlamaCppClient::set_grammar()` and cached as `Arc<String>`.
4. Every `/v1/chat/completions` request includes the grammar — the tool set is fixed for the session.

## Token metrics (`engine/token_metrics.rs`)

`watch::Sender` / `Receiver` for last prompt/generated token counts. Both `OllamaClient` and `LlamaCppClient` publish snapshots for the UI header.

## Reasoning router (`engine/router.rs`)

`ReasoningRouter` is a small **FSM** for `<redacted_thinking>` tag stripping. It is **only referenced from unit tests** — not wired into either client or the orchestrator. Config still carries `enable_reasoning_fsm` but production streaming does not invoke this FSM.

## Cosine similarity

Used in `tool_router.rs` for embedding similarity (not re-exported as a shared util).

## Testing

- `engine/ollama.rs` tests use **wiremock** for HTTP failure paths; live Ollama not required.
- `engine/grammar/` has **35 tests** covering: envelope compilation, tool name generation, JSON Schema to GBNF translation for all tool types, round-trip validation, graceful fallback for unsupported schemas.
