# Engine: LLM and auxiliary routing

## Trait (`engine/traits.rs`)

`LlmEngine::generate(stack, available_tools_json, stream_tx)`:

- **`stack`:** `&[Message]` with roles `system` | `user` | `assistant`.
- **`available_tools_json`:** second argument; **`OllamaClient` currently injects tools from the first system message** and passes `""` from orchestrator (tools live inside the assembled system prompt from `orchestrator::context::ContextAssembler`, between FCP markers in `context/view.rs`).
- **`stream_tx`:** optional; TUI can stream tokens (not all paths enable it).

## Ollama client (`engine/ollama.rs`)

- Builds `ChatMessageRequest` with `FormatType::Json` for structured assistant output.
- `GenerationOptions::num_ctx` from `AppConfig`.
- Timeouts on connect and stream chunks.
- On success, publishes `LlmTokenSnapshot` via `token_metrics` watch channel for the UI header.

## Token metrics (`engine/token_metrics.rs`)

`watch::Sender` / `Receiver` for last prompt/generated token counts for display.

## Reasoning router (`engine/router.rs`)

`ReasoningRouter` is a small **FSM** over stream chunks to strip or segment `<redacted_thinking>`…`</redacted_thinking>`. It is **not** the same as ToolRouter. As of the current tree it is **only referenced from unit tests inside `engine/router.rs`**—it is not wired into `OllamaClient` or the orchestrator. Config still carries `enable_reasoning_fsm`, but production streaming does not invoke this FSM yet.

**Mental model:** “Tool routing” = embeddings + ToolRouter; “reasoning routing” = optional future tag stripping in the engine module.

## Cosine similarity

Used in `tool_router.rs` for embedding similarity (not re-exported as a shared util in a separate file).

## Testing

`engine/ollama.rs` tests use **wiremock** for HTTP failure paths; live Ollama is not required for unit tests.
