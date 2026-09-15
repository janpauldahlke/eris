# OpenRouter Setup (Hosted Chat Backend)

OpenRouter is Eris's third chat backend: a hosted, OpenAI-compatible `/chat/completions`
API in front of many frontier models. No local chat daemon is needed — you trade local
privacy for speed and capability. **Embeddings always stay local** (Ollama or llama.cpp),
because OpenRouter has no embeddings endpoint.

> **Privacy first:** routing chat to OpenRouter sends your vault content, tool outputs,
> memories, and condensation summaries to OpenRouter and its upstream model providers.
> Eris refuses to send a single request until `consent_acknowledged = true` (set by an
> explicit ignition prompt or by hand). By default Eris also sends
> `provider.data_collection = "deny"` so requests are only routed to providers that
> promise not to retain/train on your data.

## 1. Get a key

1. Create an account at [openrouter.ai](https://openrouter.ai) and add credits.
2. Create an API key (Settings → Keys).
3. Export it in your shell — **the key is never written to `config.toml`, snapshots, or
   logs**; it is read from the environment at startup:

```bash
export OPENROUTER_API_KEY=sk-or-...
```

Add it to your shell profile (`~/.zshrc`) so preflight finds it on every launch. A
different variable name can be configured via `api_key_env`.

## 2. Run ignition (recommended)

`eris` first-run ignition offers `OpenRouter` in the backend select and walks you through:

- **Model id** — curated fast/capable picks (`google/gemini-2.5-flash`,
  `openai/gpt-4o-mini`, `anthropic/claude-3.5-haiku`, `deepseek/deepseek-chat`) or any
  other `vendor/model` id.
- **Context window (`num_ctx`)** — prefilled per known model. There is no local server,
  but `num_ctx` still drives the orchestrator's budgets and the rolling-condensation
  ceiling, so it must match the hosted model's real context window.
- **Reasoning** — off by default; `low`/`medium`/`high` enable hosted reasoning
  (higher latency + reasoning tokens are billed).
- **Consent gate** — the explicit acknowledgment described above.
- **Embed backend** — a nested sub-wizard for the *local* embedding side: Ollama
  (embed model name, default `nomic-embed-text`) or llama.cpp (build dir + embed GGUF).

## 3. Config reference (`.fcp/config.toml`)

```toml
llm_backend = "OpenRouter"
embed_backend = "Ollama"          # or "LlamaCpp"; embeddings stay local
model_name = "google/gemini-2.5-flash"
num_ctx = 1000000                 # hosted model's context window

[openrouter]
model = "google/gemini-2.5-flash"
base_url = "https://openrouter.ai/api/v1"   # also works for LiteLLM-style gateways
api_key_env = "OPENROUTER_API_KEY"          # env var NAME, never the key itself
consent_acknowledged = true                 # no request is sent while false
response_format_mode = "JsonSchema"         # "JsonSchema" | "JsonObject" | "Off"
require_parameters = true                   # only route to providers honoring response_format
data_collection = "Deny"                    # "Deny" (default) | "Allow"
max_tokens = 4096
max_retries = 3
request_timeout_secs = 300
stream_idle_timeout_secs = 90
# reasoning = { Effort = "medium" }         # or { MaxTokens = 2048 }; default Off
# fallback_models = ["openai/gpt-4o-mini"]  # OpenRouter `models` routing fallback
# referer = "https://example.org"           # optional attribution header (HTTP-Referer)
# title = "Eris"                            # optional attribution header (X-Title)
# price_per_mtok_in = 0.30                  # optional local cost math (USD per 1M tokens)
# price_per_mtok_out = 2.50                 #   else Eris uses OpenRouter's reported cost
# seed = 42
# top_p = 0.95
# stop = ["<END>"]
```

## 4. Native tool calling & fallback ladder

OpenRouter turns attach OpenAI-native `tools[]` / `tool_choice` built from the **same**
offered-tool schemas llama.cpp compiles into GBNF. The model returns
`message.tool_calls`; Eris projects those into the FCP envelope and, on the next hop,
sends `role: "tool"` result frames with matching `tool_call_id`.

The model must advertise `supported_parameters=tools` (OpenRouter model page / `/models`).
When `tools` are attached, Eris does **not** also send the strict envelope
`response_format` — they are mutually exclusive on a given request.

If a model rejects `tools` with HTTP 400, Eris disables native tools for the rest of
the session and retries with the envelope `response_format` ladder:

1. **`native_tools`** (default) — `tools[]` + `tool_choice` (`required` when the router is confident, else `auto`).
2. **`json_schema`** — `response_format: { type: "json_schema", strict: true, ... }` from the offered-tool subset.
3. **`json_object`** — valid JSON guaranteed, shape enforced by prompt + recovery loop.
4. **`off`** — prompt-only, identical to the Ollama recovery behavior.

`system:health` reports the live session mode as one of those four labels.
`require_parameters = true` prevents the router from silently sending the request to a
provider that ignores `tools` / `response_format` (which would return unconstrained
prose at HTTP 200).

Config `response_format_mode` (`JsonSchema` | `JsonObject` | `Off`) is the **envelope
fallback** starting rung after a native-tools 400, not the first request.

## 5. Cost accounting

The token status line and `system:health` show per-turn and session cost. Streaming
requests include `usage`; cost comes from OpenRouter's reported credits, or from
`price_per_mtok_in`/`price_per_mtok_out` when set. Reasoning tokens are reported
separately. Rolling condensation still runs (it *reduces* paid prompt tokens on every
subsequent turn) and its summarizer call is billed and accounted like any other turn.

## 6. Troubleshooting

| Symptom | Meaning | Fix |
|---|---|---|
| Preflight: key missing | `$OPENROUTER_API_KEY` unset/empty | `export OPENROUTER_API_KEY=sk-or-...` and restart |
| **401** | Invalid/revoked key | Re-issue key; check the right env var name (`api_key_env`) |
| **402** | Out of credits | Top up at openrouter.ai |
| **429** | Rate limited | Eris retries with backoff honoring `Retry-After`; persistent 429 = raise limits or slow down |
| 400 on first tool turn | Model rejects native `tools` | Automatic: session drops to envelope `json_schema`, then `json_object`, then `Off`; pick a model with `supported_parameters=tools` |
| 400 on envelope turn | Model rejects strict `json_schema` | Automatic: Eris downgrades to `json_object`, then `Off`; or set `response_format_mode` explicitly |
| "Consent not acknowledged" | Consent gate closed | Set `consent_acknowledged = true` in `[openrouter]` after reading the privacy note above |
| Turn reports 0 tokens/cost | Provider omitted streamed usage | Rare; cost falls back to local pricing if configured |

## 7. What stays local

- **Embeddings** (ToolRouter gating + Qdrant memory) — Ollama or llama.cpp via
  `embed_backend`.
- **Vault, tools, logs** — everything except the chat/condensation prompts sent to the
  hosted model.
- **The API key** — environment only; asserted by tests to never appear in config
  serialization or `system:health` output.
