# Phase 5 — Recovery Redesign for Grammar Path

**Depends on:** Phase 4 (grammar eliminates JSON parse failures), Phase 1 (LlamaCppClient), Phase 0 (backend enum)
**Unlocks:** Phase 6 (health/tracing), Phase 7 (per-tool grammar can further reduce recovery)
**Estimated scope:** ~120 LOC new, ~80 LOC modified, ~10 tests

---

## 5.1 — Goal

With GBNF grammar enforcement (Phase 4), JSON parse failures become **structurally impossible** on the llama.cpp path. The recovery taxonomy must split: Ollama keeps its existing `RecoverFromFuckup` path unchanged, while llama.cpp gets a streamlined recovery that recognizes this class of errors is eliminated.

---

## 5.2 — Current Recovery Architecture

The recovery flow has three actors:

### 5.2.1 `llm_directive.rs` — `process_llm_response()`

Calls `parse_llm_response_protocol(raw_json)`. On parse failure → `LoopDirective::RecoverFromFuckup(msg)`.
On success but semantic violations (empty action in tool mode, Idle without message) → also `RecoverFromFuckup`.

### 5.2.2 `directive_policy.rs` — `decide_transition_from_directive()`

Maps `LoopDirective::RecoverFromFuckup(msg)` → `StateTransition::Recover { message: "[SYSTEM OVERRIDE: FUCKUP DETECTED]...", schema_retry: false }`.

### 5.2.3 `step.rs` — the main loop

Handles `StateTransition::Recover`:
- Pushes recovery message into chat stack as system content
- Increments `self.recovery_count`
- Re-enters the loop (another LLM generation)
- Bails after `max_recovery_attempts`

### 5.2.4 `recovery_policy.rs` — `classify_tool_failure()`

After tool dispatch, classifies execution errors:
- `SchemaViolation` / `ParseFault` → `TargetedSchemaRetry` (first time) or `Recoverable`
- `ToolFault` / `Io` / `NetworkFault` → `Recoverable`
- Everything else → `Fatal`

### 5.2.5 `tool_dispatch.rs` — schema retry injection

On `TargetedSchemaRetry`, injects the tool's full JSON Schema into the chat stack as `[SYSTEM RECOVERY]` content.

---

## 5.3 — What Changes for the Grammar Path

### 5.3.1 `RecoverFromFuckup` — Eliminated

**With grammar active, these error classes cannot occur:**

| Error class | Why impossible with grammar |
|---|---|
| JSON parse failure (malformed JSON) | Grammar constrains output to valid JSON |
| Missing required keys (thought, status) | Grammar enforces all keys present |
| Invalid status value | Grammar constrains to enum |
| Trailing content after JSON | Grammar ends at closing `}` |

**Semantic violations that CAN still occur with grammar:**
- `status: "Idle"` with empty `message_to_user` (grammar allows `null`)
- `status: "Reflect"` with empty `tool_calls` and empty `message_to_user`
- Tool mode empty action (Task/Reflect with no tools and no message)

These are currently caught in `directive_from_parsed()` and routed to `RecoverFromFuckup`. For the grammar path, these should be **rare** (the grammar guides the model strongly), but they're still possible. The recovery message must be adapted.

### 5.3.2 Strategy: Defensive RecoverFromFuckup for Grammar Path

Rather than removing `RecoverFromFuckup` from the code, **change its behavior when grammar is active:**

1. If `parse_llm_response_protocol` fails (should be impossible) → log at `error!` level, treat as fatal. This indicates a grammar bug, not model drift.
2. If semantic violations fire → keep the recovery, but use a **shorter, grammar-aware** message (no JSON examples needed — the grammar handles structure).

### 5.3.3 `TargetedSchemaRetry` — Redesigned

Current behavior: On `SchemaViolation`, inject the tool's raw JSON Schema as a `[SYSTEM RECOVERY]` system message. The LLM sees a wall of JSON Schema and is asked to fix its args.

**Problem with grammar path:** The model's output is grammar-constrained. Injecting raw JSON Schema as a system message works (it's input, not output), but it's wasteful and harder for smaller models to parse.

**New behavior for grammar path:** Replace raw JSON Schema injection with **natural-language arg description:**

```
Tool "vault:write" rejected your arguments.

Error: missing required field "mode".

Expected arguments:
- relative_path (string, required): path relative to vault root
- content (string, required): the text to write
- mode (string, required): "overwrite" or "append"

Retry with corrected tool_calls.
```

This is more compact and more model-friendly. The grammar still constrains the output structure — only the arg values need fixing.

---

## 5.4 — Implementation Plan

### 5.4.1 Add `backend` Context to Recovery Paths

**`src/orchestrator/core/orchestrator.rs`** (or wherever `Orchestrator` is defined):

The `Orchestrator` already has access to `config` which contains `llm_backend`. No new field needed — just use `self.config.is_llamacpp()` in recovery decision points.

### 5.4.2 Modify `llm_directive.rs`

In `process_llm_response()`:

```rust
pub fn process_llm_response(&mut self, response_json: &str) -> LoopDirective {
    match parse_llm_response_protocol(response_json) {
        Ok(parsed) => self.directive_from_parsed(parsed),
        Err(e) => {
            if self.config.is_llamacpp() {
                // Grammar bug — this should never happen with a valid grammar
                tracing::error!(
                    error = %e,
                    "GRAMMAR BUG: LLM response failed JSON parse despite active GBNF grammar"
                );
                // Still attempt recovery, but log the anomaly
            }
            LoopDirective::RecoverFromFuckup(
                llm_json_parse_recovery_message_with_excerpt(&e, response_json),
            )
        }
    }
}
```

In `directive_from_parsed()`, the semantic violation messages (empty action, Idle without message) can be **simplified** for the grammar path since the model doesn't need JSON structure reminders:

```rust
// Grammar path: shorter recovery messages (no JSON examples needed)
if self.config.is_llamacpp() {
    return LoopDirective::RecoverFromFuckup(
        "Empty action: include tool_calls or a non-empty message_to_user.".to_string(),
    );
}
// Ollama path: existing verbose messages unchanged
```

### 5.4.3 New Function: Natural-Language Schema Description

**`src/orchestrator/llm_support/json_envelope.rs`** — add:

```rust
/// Build a natural-language description of a tool's expected arguments.
/// Used for grammar-path schema recovery (instead of raw JSON Schema).
pub fn natural_language_schema_description(
    tool_name: &str,
    schema: &schemars::schema::RootSchema,
    error_message: &str,
) -> String
```

This function:

1. Extracts the `properties` from the root schema object
2. Extracts `required` field list
3. For each property, formats: `- {name} ({type}, {required|optional}): {description}`
4. Handles nested objects with indentation (one level deep only — deeper nesting is rare in tool args)
5. Handles enums: `"overwrite" or "append"`
6. Wraps in the template:

```
Tool "{tool_name}" rejected your arguments.

Error: {error_message}

Expected arguments:
{formatted_fields}

Retry with corrected tool_calls.
```

### 5.4.4 Modify Tool Dispatch Schema Recovery

**`src/orchestrator/core/tool_dispatch.rs`** — in the `TargetedSchemaRetry` handler:

```rust
ToolFailureAction::TargetedSchemaRetry => {
    let schema = gatekeeper.get_tool_schema(&tool_name);
    let recovery_msg = if self.config.is_llamacpp() {
        // Natural language description for grammar path
        natural_language_schema_description(&tool_name, &schema, &error_message)
    } else {
        // Raw JSON Schema for Ollama path (existing behavior)
        format!("[SYSTEM RECOVERY] Tool schema fault for {tool_name}:\n{schema_json}")
    };
    // Push into chat stack as system message
    // ... (existing push logic)
}
```

### 5.4.5 Modify `recovery_policy.rs`

The `classify_tool_failure` function itself doesn't need to change — it's pure policy that doesn't know about backends. The backend-awareness is in the **consumer** of its output (tool_dispatch.rs).

However, add a test documenting that the classifier is backend-agnostic by design.

### 5.4.6 SkillGuidance (New Recovery Tier — Optional Enhancement)

The metaplan mentions a `SkillGuidance` tier: when a tool with `suggested_skills` in its descriptor fails, load the skill body as recovery guidance.

Current tools with `suggested_skills`:
- `db:find_connections` → `"db-connections-recovery"`
- `mail:write` → `"mail-recipient-verify"`

**Implementation:** In tool_dispatch.rs, after a `Recoverable` failure:

```rust
if self.config.is_llamacpp() {
    if let Some(skill_id) = descriptor_registry.suggested_skill_for(&tool_name) {
        if let Ok(skill_body) = skills_reader.read(skill_id).await {
            // Append skill guidance to the recovery message
            recovery_msg.push_str(&format!("\n\n[SKILL GUIDANCE: {skill_id}]\n{skill_body}"));
        }
    }
}
```

**This is optional for Phase 5.** It can be deferred or added as a follow-up. The core recovery redesign works without it.

---

## 5.5 — Recovery Message Format Comparison

### Ollama Path (Unchanged)

```
[SYSTEM OVERRIDE: FUCKUP DETECTED] Invalid LLM Output: expected ',' or '}'
at line 5 column 12

[FCP JSON REPAIR]
The reply you just generated was not valid protocol JSON...
(full JSON examples, brace hints, etc.)

[FCP: protocol_preview]
{"thought":"I need to...
```

### Grammar Path (New)

JSON parse errors: **should never fire.** If they do:

```
[SYSTEM OVERRIDE: FUCKUP DETECTED] [GRAMMAR BUG] Invalid LLM Output: ...
```

Semantic violations:

```
[SYSTEM OVERRIDE: FUCKUP DETECTED] Empty action: include tool_calls or a non-empty message_to_user.
```

Schema retry:

```
Tool "vault:write" rejected your arguments.

Error: missing required field "mode".

Expected arguments:
- relative_path (string, required): path relative to vault root
- content (string, required): the text to write
- mode (string, required): "overwrite" or "append"

Retry with corrected tool_calls.
```

---

## 5.6 — Step.rs Trailing Content Handling

In `step.rs`, there's a check for trailing content after the JSON:

```rust
if trailing_content_after_valid_llm_json(&response.content) {
    // RecoverFromFuckup with trailing content message
}
```

**With grammar active:** This check is redundant — the grammar prevents trailing content. Skip it:

```rust
if !self.config.is_llamacpp() && trailing_content_after_valid_llm_json(&response.content) {
    // Only on Ollama path
}
```

---

## 5.7 — Tests

| # | Test name | Location | What it validates |
|---|-----------|----------|-------------------|
| 1 | `grammar_path_json_parse_error_logs_error` | `llm_directive.rs` | Parse failure with `is_llamacpp()` logs at error level |
| 2 | `grammar_path_semantic_violation_short_message` | `llm_directive.rs` | Empty action recovery message is shorter than Ollama path |
| 3 | `ollama_path_recovery_unchanged` | `llm_directive.rs` | Ollama path produces same verbose messages as before |
| 4 | `natural_language_schema_simple_tool` | `json_envelope.rs` | vault:write schema → readable description with all fields |
| 5 | `natural_language_schema_optional_fields` | `json_envelope.rs` | Tool with optional args marks them correctly |
| 6 | `natural_language_schema_enum_field` | `json_envelope.rs` | Enum values listed inline (e.g., "overwrite" or "append") |
| 7 | `natural_language_schema_empty_args` | `json_envelope.rs` | Tool with no args → "No arguments required." |
| 8 | `targeted_retry_uses_natural_language_for_llamacpp` | `tool_dispatch.rs` | Grammar path gets NL description, not raw JSON Schema |
| 9 | `targeted_retry_uses_json_schema_for_ollama` | `tool_dispatch.rs` | Ollama path unchanged |
| 10 | `trailing_content_check_skipped_for_grammar` | `step.rs` | Grammar path doesn't trigger trailing content recovery |

---

## 5.8 — Files Summary

| File | Action | What changes |
|------|--------|-------------|
| `src/orchestrator/core/llm_directive.rs` | Modify | Grammar-aware parse failure handling, shorter semantic violation messages |
| `src/orchestrator/llm_support/json_envelope.rs` | Modify | Add `natural_language_schema_description()` |
| `src/orchestrator/core/tool_dispatch.rs` | Modify | Branch schema retry message format on backend |
| `src/orchestrator/core/step.rs` | Modify | Skip trailing content check when grammar active |
| `src/orchestrator/context/resolved_tool_recovery/markers.rs` | No change | Markers stay the same (grammar path still uses FUCKUP prefix for compatibility) |
| `src/orchestrator/loop/recovery_policy.rs` | No change | Classifier is backend-agnostic |

---

## 5.9 — Acceptance Criteria

- [ ] Grammar path never produces `RecoverFromFuckup` for JSON parse errors in normal operation
- [ ] If a grammar bug causes a parse error, it's logged at `error!` level with "GRAMMAR BUG" tag
- [ ] Semantic violations (empty action) produce short, focused recovery messages on grammar path
- [ ] Schema retry uses natural-language descriptions on grammar path
- [ ] Ollama path recovery is completely unchanged (byte-identical behavior)
- [ ] Recovery count and max_recovery_attempts still work correctly on both paths
- [ ] Trailing content check is skipped on grammar path
- [ ] All existing tests pass (no regression on Ollama recovery)
- [ ] 10 new tests covering the split behavior

---

## 5.10 — Risk Assessment

**Low risk:** This phase is mostly additive branching (`if is_llamacpp()`) with a new helper function. The Ollama path is untouched by design.

**Medium risk:** The `natural_language_schema_description` function must handle the full diversity of `RootSchema` structures from `schemars`. Edge cases:
- Nested objects (e.g., `moltbook:dm` has complex arg variants)
- `oneOf` / `anyOf` schemas
- Array-typed fields with item schemas

**Mitigation:** For any schema construct the NL builder can't handle, fall back to dumping the raw JSON Schema fragment for that field. Graceful degradation, not hard failure.
