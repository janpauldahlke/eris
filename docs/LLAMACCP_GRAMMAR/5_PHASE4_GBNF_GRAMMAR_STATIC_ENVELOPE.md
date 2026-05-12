# Phase 4 — GBNF Grammar: Static Envelope

**Depends on:** Phase 1 (LlamaCppClient to receive the grammar), Phase 0 (types)
**Unlocks:** Phase 5 (recovery redesign), Phase 7 (per-tool arg grammar)
**Estimated scope:** ~300 LOC new, ~30 LOC modified, ~12 tests
**Risk level:** HIGH — grammar correctness is critical; overtight grammar breaks tool calling, too loose negates the value.

---

## 4.1 — Goal

Build the GBNF grammar that constrains LLM output to **valid FCP protocol JSON**. After this phase, every LLM response from the llama.cpp path is guaranteed to parse as `LlmResponse` — eliminating the `RecoverFromFuckup` error class entirely.

The grammar constrains the **envelope** (status, thought, message_to_user, tool_calls structure, tool name enum) but leaves `args` as **freeform JSON object**. This is the pragmatic split: envelope correctness eliminates 90%+ of parse failures; arg validation stays in the Gatekeeper.

---

## 4.2 — GBNF Primer

GBNF (GGML BNF) is llama-server's grammar format. Key syntax:

```
rule-name ::= expression
"literal" — exact string match
[a-z] — character class
rule1 | rule2 — alternation
rule* — zero or more
rule+ — one or more
rule? — optional
(group) — grouping
```

llama-server accepts a `grammar` field in the completion request body. When set, the model's output is constrained token-by-token to match the grammar.

---

## 4.3 — New Module: `src/engine/grammar/`

### 4.3.1 File Layout

```
src/engine/grammar/
├── mod.rs          — public API: compile_fcp_envelope_grammar()
├── envelope.rs     — static GBNF rules for the protocol shape
└── tool_names.rs   — dynamic tool name enum builder
```

### 4.3.2 `mod.rs`

```rust
mod envelope;
mod tool_names;

pub use envelope::compile_fcp_envelope_grammar;
```

### 4.3.3 Register in `src/engine/mod.rs`

```rust
pub mod grammar;
```

---

## 4.4 — The Grammar Specification

### 4.4.1 Target JSON Shape (from `LlmResponse`)

The grammar must produce exactly this structure:

```json
{
  "thought": "...",
  "status": "Task" | "Reflect" | "Idle" | "Process",
  "message_to_user": "..." | null,
  "tool_calls": [
    {
      "name": "<tool_name>",
      "args": { ... any JSON object ... }
    }
  ]
}
```

Field ordering in JSON doesn't matter for `serde`, but GBNF grammars typically enforce a fixed key order to keep the grammar tractable. **Enforce this order:** `thought`, `status`, `message_to_user`, `tool_calls`.

### 4.4.2 Complete Static GBNF (Template)

```gbnf
root ::= "{" ws thought-kv "," ws status-kv "," ws message-kv "," ws toolcalls-kv ws "}"

ws ::= [ \t\n]*

thought-kv ::= "\"thought\"" ws ":" ws json-string
status-kv  ::= "\"status\"" ws ":" ws status-enum
message-kv ::= "\"message_to_user\"" ws ":" ws (json-string | "null")
toolcalls-kv ::= "\"tool_calls\"" ws ":" ws "[" ws tool-call-list ws "]"

status-enum ::= "\"Task\"" | "\"Reflect\"" | "\"Idle\"" | "\"Process\""

tool-call-list ::= "" | tool-call ("," ws tool-call)*

tool-call ::= "{" ws "\"name\"" ws ":" ws tool-name-enum ws "," ws "\"args\"" ws ":" ws json-object ws "}"

# Dynamic — injected at compile time:
tool-name-enum ::= "\"vault:read\"" | "\"vault:write\"" | ...

# JSON primitives for freeform args:
json-object ::= "{" ws json-members? ws "}"
json-members ::= json-pair ("," ws json-pair)*
json-pair ::= json-string ws ":" ws json-value
json-value ::= json-string | json-number | json-object | json-array | "true" | "false" | "null"
json-array ::= "[" ws json-elements? ws "]"
json-elements ::= json-value ("," ws json-value)*
json-string ::= "\"" json-char* "\""
json-char ::= [^"\\] | "\\" json-escape
json-escape ::= ["\\nrtbf/] | "u" [0-9a-fA-F] [0-9a-fA-F] [0-9a-fA-F] [0-9a-fA-F]
json-number ::= "-"? ("0" | [1-9] [0-9]*) ("." [0-9]+)? ([eE] [+-]? [0-9]+)?
```

### 4.4.3 Key Design Decisions

1. **Fixed key order.** The grammar enforces `thought → status → message_to_user → tool_calls`. This is necessary because GBNF cannot express "any permutation of N keys" without exponential rule explosion. Since we control the system prompt, we instruct the model to use this order.

2. **`message_to_user` is always present.** Rather than making the key optional (complex in GBNF), require it with `null` when empty. This simplifies the grammar significantly. `LlmResponse` already handles `Option<String>` via serde — `null` deserializes to `None`.

3. **`tool_calls` is always present.** Empty array `[]` when no tools. Already matches current behavior.

4. **`args` is freeform `json-object`.** Not constrained per-tool. Phase 7 tightens this.

5. **No trailing content.** The grammar ends at the closing `}` of root — no markdown, no prose after the JSON. This eliminates the `trailing_content_after_valid_llm_json` error class.

---

## 4.5 — `envelope.rs` Implementation

### 4.5.1 Public Function

```rust
/// Build a complete GBNF grammar string for the FCP protocol envelope.
/// `tool_names` is the set of registered tool names (e.g., ["vault:read", "vault:write", ...]).
/// Returns a ready-to-use GBNF string.
pub fn compile_fcp_envelope_grammar(tool_names: &[String]) -> String
```

### 4.5.2 Construction Strategy

1. Start with the **static parts** (everything except `tool-name-enum`) as a `const &str` template.
2. Build the `tool-name-enum` rule dynamically from the `tool_names` slice (see §4.6).
3. Concatenate into the final grammar string.
4. If `tool_names` is empty, the `tool-call-list` rule becomes `""` only (no tool calls possible), and `tool-name-enum` is omitted. This covers the conversational-only edge case.

### 4.5.3 Grammar String Escaping

Tool names contain `:` (e.g., `vault:read`). In GBNF, these go inside double quotes: `"\"vault:read\""`. The builder must properly escape any special characters, though in practice tool names only contain `[a-z:_]`.

---

## 4.6 — `tool_names.rs` Implementation

### 4.6.1 Public Function

```rust
/// Build the GBNF alternation rule for valid tool names.
/// Returns a string like: `"\"vault:read\"" | "\"vault:write\"" | ...`
pub fn build_tool_name_enum(tool_names: &[String]) -> String
```

### 4.6.2 Logic

```rust
tool_names.iter()
    .map(|name| format!("\"\\\"{}\\\"\"", name))
    .collect::<Vec<_>>()
    .join(" | ")
```

Each name becomes a GBNF literal matching the JSON string `"tool_name"`.

---

## 4.7 — Wiring the Grammar into `LlamaCppClient`

### 4.7.1 Grammar Cache

The tool set is fixed at session start (tools are registered before the first `step()`). The grammar should be compiled once and cached:

```rust
pub struct LlamaCppClient {
    http: reqwest::Client,
    chat_url: String,
    config: Arc<AppConfig>,
    token_metrics_tx: Option<...>,
    grammar: Option<String>,    // NEW: cached GBNF
}
```

### 4.7.2 Grammar Injection

Add a method:
```rust
pub fn set_grammar(&mut self, grammar: String) {
    self.grammar = Some(grammar);
}
```

### 4.7.3 Request Body Update

In the `generate` method, add the grammar to the request:

```rust
let request = ChatCompletionRequest {
    messages: ...,
    stream: ...,
    temperature: ...,
    n_predict: ...,
    grammar: self.grammar.clone(),  // was: None in Phase 1
};
```

### 4.7.4 When to Compile

In `chat_session.rs`, **before** the engine is moved into `Orchestrator::new(engine, ...)`. The sequencing is critical: `set_grammar` needs `&mut self` on the concrete `LlamaCppClient`, which is impossible after it's boxed/moved.

```rust
// After gatekeeper has all tools registered, before Orchestrator::new
if config.is_llamacpp() {
    let tool_names = gatekeeper.registered_tool_names();
    let grammar = compile_fcp_envelope_grammar(&tool_names);
    llamacpp_client.set_grammar(grammar);  // must happen before move into Orchestrator
}
let engine = /* box or wrap llamacpp_client */;
let mut orchestrator = Orchestrator::new(engine, gatekeeper, ...);
```

Alternative: pass the grammar through the constructor (`LlamaCppClient::new(config, grammar)`) or make the grammar an `Option` set during construction. Either way, it must be set before the engine is consumed by the orchestrator.

---

## 4.8 — System Prompt Adjustment

When grammar is active, the system prompt must instruct the model to use the **exact field order** the grammar expects. Add a section to the system prompt assembly (in `ContextAssembler` or wherever the protocol instructions live):

```
CRITICAL: Your response must be a single JSON object with keys in this exact order:
1. "thought" (string)
2. "status" ("Task", "Reflect", "Idle", or "Process")
3. "message_to_user" (string or null)
4. "tool_calls" (array, may be empty [])

Do not include any text before or after the JSON object.
```

This is only injected when `config.is_llamacpp()`. The Ollama path keeps its existing protocol instructions.

---

## 4.9 — Edge Cases and Pitfalls

### 4.9.1 Empty Tool Names

If no tools are registered (unlikely but possible in test scenarios), the grammar should still work: `tool-call-list` allows empty array, and `tool-name-enum` is never referenced. Handle by conditionally including/excluding the `tool-call` and `tool-name-enum` rules.

### 4.9.2 Very Long Tool Name Lists

With 40+ tools, the `tool-name-enum` rule becomes long but is still O(n) in grammar size. No performance concern.

### 4.9.3 JSON String Escaping in `thought` and `message_to_user`

The model's thoughts and messages can contain arbitrary text including quotes, newlines, backslashes. The `json-string` rule with proper escape handling covers this. Test with adversarial content.

### 4.9.4 Unicode in JSON Strings

The `json-char` rule allows any character except `"` and `\`. Unicode outside ASCII is valid in JSON strings without escaping. GBNF handles this via the character class negation `[^"\\]`.

### 4.9.5 Numeric Args

Tool args like `{ "minutes": 30 }` require `json-number` in the freeform `json-value` rule. Ensure the grammar includes proper number parsing (integer and float).

---

## 4.10 — Tests

### 4.10.1 Grammar Compilation Tests

| # | Test name | What it validates |
|---|-----------|-------------------|
| 1 | `compile_with_tools_produces_valid_gbnf` | Grammar string is non-empty, contains all tool names, has `root ::=` |
| 2 | `compile_with_empty_tools` | No tool names → grammar still valid (empty tool_calls only) |
| 3 | `tool_name_enum_formats_correctly` | `["vault:read", "vault:write"]` → `"\"vault:read\"" \| "\"vault:write\""` |
| 4 | `grammar_contains_all_status_values` | Output contains `Task`, `Reflect`, `Idle`, `Process` |

### 4.10.2 Grammar Validation Tests (Parse Representative Outputs)

Use the compiled grammar to validate that representative LLM outputs match. Since we can't run the grammar engine in-process easily, these tests validate **indirectly**: compile the grammar, then parse known-good JSON through `LlmResponse` serde, and verify the JSON string structure matches the grammar's constraints.

| # | Test name | What it validates |
|---|-----------|-------------------|
| 5 | `idle_response_matches_grammar_shape` | `{"thought":"...","status":"Idle","message_to_user":"hi","tool_calls":[]}` parses |
| 6 | `task_with_tool_call_matches` | Response with one tool call, string args |
| 7 | `task_with_numeric_args_matches` | Tool call with `{"minutes": 30}` |
| 8 | `task_with_nested_args_matches` | Tool call with nested object args |
| 9 | `task_with_multiple_tool_calls_matches` | Two tool calls in one response |
| 10 | `null_message_matches` | `"message_to_user": null` |
| 11 | `thought_with_special_chars_matches` | Thought containing quotes, newlines, backslashes |
| 12 | `process_status_alias_matches` | `"status": "Process"` is valid |

### 4.10.3 Negative Tests

If feasible, verify that strings violating the grammar (trailing prose, missing fields, wrong key order) would NOT be produced. These are assertions on the grammar's design intent rather than runtime enforcement (llama-server enforces at runtime).

---

## 4.11 — Files Summary

| File | Action | What changes |
|------|--------|-------------|
| `src/engine/grammar/mod.rs` | Create | Module declaration, public API |
| `src/engine/grammar/envelope.rs` | Create | `compile_fcp_envelope_grammar()` |
| `src/engine/grammar/tool_names.rs` | Create | `build_tool_name_enum()` |
| `src/engine/mod.rs` | Modify | Add `pub mod grammar;` |
| `src/engine/llama_cpp.rs` | Modify | Add `grammar` field, `set_grammar()`, include in request body |
| `src/executive/chat_session.rs` | Modify | Compile and set grammar after tool registration |
| Context assembler (wherever prompt is built) | Modify | Add field-order instruction for llama.cpp path |

---

## 4.12 — Acceptance Criteria

- [ ] Grammar compiles for the full tool roster (40+ tools)
- [ ] Grammar string is syntactically valid GBNF (no llama-server parse errors)
- [ ] llama-server with grammar produces JSON that parses as `LlmResponse` on every response
- [ ] Tool calls with various arg types (string, number, boolean, nested object, array) work
- [ ] `message_to_user: null` works for tool-only responses
- [ ] No trailing prose after JSON (grammar prevents it)
- [ ] Ollama path completely unaffected (no grammar sent)
- [ ] System prompt includes field-order instructions for llama.cpp path
