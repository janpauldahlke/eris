# Phase 7 — Dynamic Per-Tool Arg Grammar (Stretch)

**Depends on:** Phase 4 (static envelope grammar), Phase 5 (recovery handles arg failures)
**Unlocks:** Nothing (terminal stretch goal)
**Estimated scope:** ~400 LOC new, ~20 LOC modified, ~15 tests
**Risk level:** HIGHEST — JSON Schema to GBNF translation is a non-trivial compiler problem.

---

## 7.1 — Goal

Tighten the GBNF grammar so `args` is constrained **per tool**, eliminating schema validation failures entirely. After this phase, the Gatekeeper's `validate_args` should never reject tool calls from the llama.cpp path — the grammar structurally prevents invalid arguments.

This is the hard phase. It should only proceed after Phases 0-6 are stable and battle-tested. **The Phase 4 freeform-args grammar is a fully functional fallback.**

---

## 7.2 — How It Works

### 7.2.1 Current State (Phase 4)

```gbnf
tool-call ::= "{" ws "\"name\"" ws ":" ws tool-name-enum ws "," ws "\"args\"" ws ":" ws json-object ws "}"
```

`json-object` allows any valid JSON object as args. The Gatekeeper validates post-hoc.

### 7.2.2 Target State (Phase 7)

```gbnf
tool-call ::= "{" ws "\"name\"" ws ":" ws tool-with-args ws "}"

tool-with-args ::=
    "\"vault:read\""  ws "," ws "\"args\"" ws ":" ws vault-read-args
  | "\"vault:write\"" ws "," ws "\"args\"" ws ":" ws vault-write-args
  | "\"memory:stage\"" ws "," ws "\"args\"" ws ":" ws memory-stage-args
  | ...

vault-read-args ::= "{" ws "\"relative_path\"" ws ":" ws json-string ws "}"

vault-write-args ::= "{" ws
    "\"relative_path\"" ws ":" ws json-string ws ","  ws
    "\"content\"" ws ":" ws json-string ws "," ws
    "\"mode\"" ws ":" ws ("\"overwrite\"" | "\"append\"") ws
"}"
```

The tool name and its args shape are **coupled** in the grammar — choosing a tool name locks the model into that tool's exact arg structure.

---

## 7.3 — New File: `src/engine/grammar/schema_to_gbnf.rs`

### 7.3.1 Scope of JSON Schema Support

The tool schemas use `schemars::schema_for!()` which produces standard JSON Schema. The converter must handle this **bounded subset**:

| JSON Schema construct | GBNF mapping | Used by tools |
|---|---|---|
| `type: "object"` with `properties` | Named rule with fixed keys | All tools |
| `type: "string"` | `json-string` | Most fields |
| `type: "string"` with `enum` | Quoted alternation | `vault:write.mode`, `moltbook:vote.direction`, etc. |
| `type: "integer"` / `type: "number"` | `json-number` | `minutes`, `hour`, `top_k`, etc. |
| `type: "boolean"` | `"true" \| "false"` | `include_content_preview`, `permanent`, etc. |
| `type: "array"` with `items` | `"[" ws (item ("," ws item)*)? ws "]"` | `memory:stage.tags`, `agenda:remind_self.checklist` |
| `required` vs optional | Required keys always emitted; optional keys use `(key-value ",")?` | Most tools |
| `type: "null"` / nullable | `json-value \| "null"` | Some optional fields |

### 7.3.2 Unsupported Constructs (Graceful Degradation)

| JSON Schema construct | Fallback |
|---|---|
| `oneOf` / `anyOf` / `allOf` | Fallback to `json-object` for that tool's args |
| `additionalProperties` (free-form extra keys) | Fallback to `json-object` |
| `$ref` (internal references) | Resolve before converting; if unresolvable, fallback |
| Deeply nested objects (>2 levels) | Emit `json-object` for nested sub-objects beyond depth 2 |
| `pattern` (regex on strings) | Ignore constraint, use `json-string` |
| `minItems` / `maxItems` | Ignore, use unconstrained array |

**Fallback strategy:** If the schema-to-GBNF compiler encounters anything it can't handle, it falls back to `json-object` **for that specific tool's args only**. Other tools still get full grammar coverage. Log a `warn!` with the tool name and unsupported construct.

### 7.3.3 Public API

```rust
/// Compile a per-tool arg rule from its JSON Schema.
/// Returns (rule_name, rule_body) or None if the schema is too complex.
pub fn schema_to_gbnf_rule(
    tool_name: &str,
    schema: &schemars::schema::RootSchema,
) -> Option<(String, String)>
```

`rule_name` is derived from tool_name: `vault:read` → `vault-read-args`. Colons become dashes.

`rule_body` is the GBNF rule definition (right side of `::=`).

### 7.3.4 Implementation Strategy

1. **Extract the root object schema:** `schema.schema.object` → properties, required.
2. **For each property:**
   - Determine its type from `instance_type`
   - If `enum_values` is present → build alternation of quoted literals
   - If `type: "array"` → recurse into `items`
   - If `type: "object"` → recurse (depth-limited)
3. **Build the rule:**
   - Required properties are emitted in fixed order (alphabetical by key name)
   - Optional properties use `( key-pair "," ws )?` syntax
   - Problem: GBNF doesn't support "any subset of optional keys in any order" cleanly

### 7.3.5 The Optional Field Problem

This is the **hardest part** of the compiler. JSON objects can have optional keys in any order. GBNF is a CFG that enforces a specific sequence. Options:

**Option A: Fixed key order, optional keys use `(key ",")?`**

```gbnf
vault-write-args ::= "{" ws
    "\"relative_path\"" ws ":" ws json-string ws "," ws
    "\"content\"" ws ":" ws json-string ws "," ws
    "\"mode\"" ws ":" ws ("\"overwrite\"" | "\"append\"")
    ( ws "," ws "\"extra_optional\"" ws ":" ws json-string )?
    ws "}"
```

This works but requires the model to emit keys in a fixed order. Since the grammar constrains output, the model adapts. This is the **recommended approach**.

**Option B: Permutation explosion**

For N optional keys, generate all N! orderings. This is exponential and unacceptable for tools with 5+ optional fields.

**Option C: All optional keys as a freeform tail**

Required keys first (grammar-constrained), then `json-object` for optional keys. Partial coverage.

**Recommendation: Option A.** Sort all keys (required first, then optional alphabetically). Enforce this fixed order in the grammar. The system prompt tells the model to use this order. Grammar-constrained models follow it naturally.

---

## 7.4 — Grammar Compilation Flow

### 7.4.1 Per-Session Compilation

The grammar is compiled once per session (tool set is fixed). The flow:

```
Gatekeeper.registered_tools()
    → for each tool:
        tool.parameters_schema()
        → schema_to_gbnf_rule(name, schema)
            → Some((rule_name, rule_body))  // or None for fallback
    → assemble into tool-with-args alternation
    → merge with static envelope rules
    → final GBNF string
```

### 7.4.2 Modified `compile_fcp_envelope_grammar`

The function signature changes to accept schemas:

```rust
pub fn compile_fcp_envelope_grammar(
    tools: &[(String, Option<(String, String)>)],  // (name, Some((rule_name, rule_body)) or None)
) -> String
```

Tools with `None` (fallback) use the generic `json-object` for args. Tools with `Some` get their specific rule.

The `tool-with-args` alternation becomes:

```gbnf
tool-with-args ::=
    "\"vault:read\"" ws "," ws "\"args\"" ws ":" ws vault-read-args
  | "\"vault:write\"" ws "," ws "\"args\"" ws ":" ws vault-write-args
  | "\"memory:stage\"" ws "," ws "\"args\"" ws ":" ws json-object   # fallback
  | ...
```

---

## 7.5 — Example Tool Translations

### 7.5.1 `vault:read` (Simple: one required string)

Schema: `{ "relative_path": string (required) }`

```gbnf
vault-read-args ::= "{" ws "\"relative_path\"" ws ":" ws json-string ws "}"
```

### 7.5.2 `vault:write` (Three required strings, one is enum)

Schema: `{ "relative_path": string, "content": string, "mode": enum("overwrite","append") }` — all required.

```gbnf
vault-write-args ::= "{" ws
    "\"content\"" ws ":" ws json-string ws "," ws
    "\"mode\"" ws ":" ws ("\"overwrite\"" | "\"append\"") ws "," ws
    "\"relative_path\"" ws ":" ws json-string ws
"}"
```

(Keys sorted alphabetically: content, mode, relative_path)

### 7.5.3 `memory:stage` (Mixed required + optional, array field)

Schema: `{ "title": string (req), "content": string (req), "tags": array<string> (req) }`

```gbnf
memory-stage-args ::= "{" ws
    "\"content\"" ws ":" ws json-string ws "," ws
    "\"tags\"" ws ":" ws "[" ws json-string-list ws "]" ws "," ws
    "\"title\"" ws ":" ws json-string ws
"}"

json-string-list ::= json-string ("," ws json-string)*
```

### 7.5.4 `clock:timer` (Integer + string)

Schema: `{ "minutes": integer (req), "label": string (req) }`

```gbnf
clock-timer-args ::= "{" ws
    "\"label\"" ws ":" ws json-string ws "," ws
    "\"minutes\"" ws ":" ws json-number ws
"}"
```

### 7.5.5 `agenda:remind_at` (Complex: optional fields, mixed types)

Schema has: `task_id` (optional string), `description` (optional string), `minutes` (optional int), `hour` (optional int), `minute` (optional int).

This is where the fixed-order-optional approach is tested:

```gbnf
agenda-remind-at-args ::= "{" ws
    ( "\"description\"" ws ":" ws json-string ws "," ws )?
    ( "\"hour\"" ws ":" ws json-number ws "," ws )?
    ( "\"minute\"" ws ":" ws json-number ws "," ws )?
    ( "\"minutes\"" ws ":" ws json-number ws "," ws )?
    ( "\"task_id\"" ws ":" ws json-string ws )?
ws "}"
```

**Issue:** This can produce `{}` (all optional fields absent) which is semantically invalid but syntactically valid. The Gatekeeper catches this post-hoc. Alternatively, mark at least one field as required in the grammar if the schema implies it.

**Trailing comma issue:** When the last optional field is present but the ones after it are absent, there's a trailing comma. Fix with careful grammar construction — the comma is part of the optional group:

```gbnf
# Each optional field includes its trailing comma
optional-field ::= "\"key\"" ws ":" ws value ws ","
# Last field has no comma
```

This is tricky to get right. Each optional group must know whether more fields follow. This may require generating the rule programmatically with positional awareness.

---

## 7.6 — The Trailing Comma Problem (Critical)

JSON does not allow trailing commas. If we have:

```gbnf
args ::= "{" ws
    required-a ws "," ws
    ( optional-b ws "," ws )?
    required-c ws
"}"
```

When `optional-b` is absent, we get `{ "a": 1, , "c": 3 }` — invalid JSON!

**Solution: Pair optional fields with their preceding comma:**

```gbnf
args ::= "{" ws
    required-a
    ( ws "," ws optional-b )?
    ws "," ws required-c
ws "}"
```

Now if `optional-b` is absent: `{ "a": 1, "c": 3 }` — valid.
If present: `{ "a": 1, "b": 2, "c": 3 }` — valid.

For multiple optional fields between required fields, chain them:

```gbnf
args ::= "{" ws
    required-a
    ( ws "," ws optional-b )?
    ( ws "," ws optional-c )?
    ws "," ws required-d
ws "}"
```

This means optional fields must come before the next required field. With alphabetical key ordering and mixed required/optional, the layout becomes:

1. Sort all keys alphabetically
2. Group: `required → optional → required → optional → ...`
3. First required key has no leading comma
4. Each subsequent key (required or optional) includes a leading `, `
5. Optional keys wrap the leading comma + key-value in `(...)?`

Edge case: **all fields are optional** (e.g., `system:health` with `SystemHealthArgs {}`). Grammar becomes `"{" ws "}"` — empty object.

---

## 7.7 — Tests

### 7.7.1 Compiler Unit Tests

| # | Test name | What it validates |
|---|-----------|-------------------|
| 1 | `simple_required_string_field` | Single required string → correct GBNF rule |
| 2 | `multiple_required_fields_sorted` | Three required fields → alphabetical order in grammar |
| 3 | `enum_string_field` | String with enum → alternation of quoted literals |
| 4 | `integer_field` | Integer type → `json-number` |
| 5 | `boolean_field` | Boolean → `"true" \| "false"` |
| 6 | `array_of_strings` | `Vec<String>` → `"[" ws json-string-list ws "]"` |
| 7 | `optional_field_syntax` | Optional field → `(ws "," ws ...)?` |
| 8 | `mixed_required_optional` | Required + optional → correct comma handling |
| 9 | `all_optional_fields` | All optional → no trailing comma issues |
| 10 | `empty_args` | No parameters → `"{" ws "}"` |
| 11 | `unsupported_schema_returns_none` | `oneOf` in schema → `None` (fallback) |
| 12 | `tool_name_to_rule_name` | `vault:read` → `vault-read-args` |

### 7.7.2 Integration Tests (Round-Trip)

| # | Test name | What it validates |
|---|-----------|-------------------|
| 13 | `compiled_grammar_for_full_roster_is_valid` | Compile grammar for all 40+ tools → non-empty, no panics |
| 14 | `representative_response_parses_with_typed_args` | JSON with correctly typed args → `serde_json::from_str` succeeds |
| 15 | `fallback_tools_still_accept_freeform_args` | Tools that fell back → `json-object` args still work |

---

## 7.8 — Incremental Rollout Strategy

Given the complexity, this phase should roll out incrementally:

1. **Step 1:** Implement the compiler for the simplest tools (all-required, string/number only): `vault:read`, `vault:write`, `clock:now`, `system:health`
2. **Step 2:** Add enum support: `vault:write.mode`
3. **Step 3:** Add optional field support: `memory:query`, `web:fetch`
4. **Step 4:** Add array support: `memory:stage.tags`
5. **Step 5:** Audit all 40+ tools, mark which ones can be typed and which fall back

Each step is independently testable and deployable with the Phase 4 fallback covering ungrouped tools.

---

## 7.9 — Files Summary

| File | Action | What changes |
|------|--------|-------------|
| `src/engine/grammar/schema_to_gbnf.rs` | Create | JSON Schema → GBNF compiler |
| `src/engine/grammar/mod.rs` | Modify | Add `mod schema_to_gbnf`, update public API |
| `src/engine/grammar/envelope.rs` | Modify | Accept per-tool rules, build `tool-with-args` alternation |
| `src/executive/chat_session.rs` | Modify | Pass tool schemas to grammar compiler |

---

## 7.10 — Acceptance Criteria

- [ ] Schema-to-GBNF compiler handles the bounded subset of JSON Schema
- [ ] Unsupported constructs fall back gracefully (freeform args for that tool)
- [ ] No trailing comma issues in generated grammar
- [ ] Grammar compiles for the full tool roster without errors
- [ ] llama-server accepts and enforces the generated grammar
- [ ] Tool calls from grammar-constrained output pass Gatekeeper validation
- [ ] Fallback tools still work with freeform args
- [ ] At least the 5 simplest tools have full arg grammar coverage
- [ ] All Phase 4 tests still pass (static envelope is a subset of this)
