1. Objective

   Isolate the dreadnought's text generation mechanics behind a strict asynchronous Rust trait. By decoupling the Subconscious Orchestrator (Layer 2) from the physical inference engine (Layer 1), we enable rapid V1 deployment using local HTTP daemons (Ollama) while leaving the door mathematically open for native C++ bindings or cloud providers in V2.

2. Architecture & Design Rules

A. The Engine Core (The Trait Boundary)

All text generation must pass through a strict, state-agnostic interface in `src/engine/mod.rs`. The engine does not know about the `moka` cache or the Qdrant vault; it only processes strings and JSON schemas.

Rust

```
use async_trait::async_trait;
use tokio::sync::mpsc;
use crate::error::Result;

#[derive(Debug, Clone)]
pub struct Message {
    pub role: String, // "system", "user", "assistant"
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct EngineResponse {
    pub content: String,
    pub prompt_tokens: usize,
    pub generated_tokens: usize,
}

#[async_trait]
pub trait LlmEngine: Send + Sync {
    /// Ingests the Chat Stack and Tool Schema.
    /// Streams live reasoning tokens to `think_tx`. Returns final JSON and token metrics.
    async fn generate(
        &self,
        stack: &[Message],
        available_tools_json: &str,
        think_tx: Option<mpsc::UnboundedSender<String>>
    ) -> Result<EngineResponse>;
}
```

B. The Ollama Implementation (`src/engine/ollama.rs`)

For the V1 MVP, the `LlmEngine` trait is implemented via the `ollama-rs` crate.

- **Initialization:** The `OllamaClient` struct is instantiated at boot, reading `model_name` and `ollama_host` from the injected `Arc<AppConfig>`.
- **The JSON Shield:** Ollama natively supports a `.format("json")` flag. The client MUST set this flag to force the model's output weights into strict JSON compliance.
- **Token Telemetry:** The client extracts `prompt_eval_count` and `eval_count` from the final HTTP response, packing them into the `EngineResponse` struct so Layer 2 can execute the Condensation math.

C. The Reasoning Stream (No Regex)

We rely entirely on the Engine's native API to separate the cognitive monologue from the JSON output.

- **The Intercept:** When `generate` is called with a `think_tx` channel, the `OllamaClient` sets `think: true` in the API payload and consumes the stream.
- **Routing:** As chunks arrive, any string in the `message.thinking` field is immediately pushed to the `think_tx` channel for live TUI rendering. Any string in `message.content` is buffered internally.
- **The Clean Return:** Upon stream completion, the buffered JSON string is wrapped in the `EngineResponse` and returned to the Orchestrator. No regex required.

D. Timeout & Network Handling

- The execution must be wrapped in a `tokio::time::timeout`. If Ollama hangs for longer than `AppConfig.generation_timeout_secs`, the client drops the future and yields an `Err(FcpError::EngineFault("Timeout"))`.
- If the Ollama daemon is dead, the connection failure maps instantly to `Err(FcpError::NetworkFault)`.

3. Acceptance Criteria

- [ ] The `LlmEngine` trait compiles successfully, accepting an optional `mpsc` channel for live telemetry.
- [ ] Executing `OllamaClient::generate()` successfully requests JSON formatting and returns the populated `EngineResponse`.
- [ ] When provided a `think_tx` channel, the engine successfully streams Ollama's `message.thinking` chunks to the receiver in real-time while silently buffering the final JSON payload.
- [ ] If the Ollama daemon is offline or times out, the client immediately catches the network rejection and returns the correct structured `FcpError` without panicking the thread.
