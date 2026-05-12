# Handover — Testing & Documentation Phase

**Branch:** `feature/llama_gguf_grammar`
**State:** All code committed (`3fe0ec5`), 524 tests pass, 0 failures.
**What remains:** Live testing with actual llama-server, then documentation (Phase 8).

---

## What was built (Phases 0–7)

| Phase | Commit | Summary |
|-------|--------|---------|
| 0 | `0834969` | `LlmBackend` enum, `LlamaCppConfig`, ignition wiring |
| 1 | `d1f8d0f` | `LlamaCppClient` implements `LlmEngine` via HTTP |
| 2 | `76abed2` | `PeripheralLifecycle` spawns/probes/shuts down llama-server (chat + embed) |
| 3 | `4633f0f` | `EmbeddingProvider` trait; Ollama + LlamaCpp impls; router/semantic use `Arc<dyn>` |
| 4 | `10a2179` | Static GBNF envelope grammar, `compile_fcp_envelope_grammar()`, grammar on client |
| 5 | `c755332` | Recovery split: grammar path eliminates `RecoverFromFuckup`, NL schema retry |
| 6 | `27e26de` | `system:health` backend awareness, llama-server `/health` probe, preflight branching |
| 7 | `3fe0ec5` | **Dynamic per-tool arg grammar** — JSON Schema → GBNF compiler for all 40+ tools |

### Phase 7 files

| File | Action | Lines |
|------|--------|------:|
| `src/engine/grammar/schema_to_gbnf.rs` | Created | 633 |
| `src/engine/grammar/envelope.rs` | Modified | +149 |
| `src/engine/grammar/mod.rs` | Modified | +4 |
| `src/executive/chat_session.rs` | Modified | +33 |

### Phase 7 test count: 21 new (35 total in grammar module)

---

## Testing Plan

### Step 1 — Verify Ollama path is undamaged

```bash
# Full test suite (should report 524 passed)
cargo test

# Manual smoke test with existing Ollama vault
cd /path/to/existing-ollama-vault
eris chat
# Try: "what time is it?" → clock:now
# Try: "read my identity" → vault:read
# Try: "system health" → system:health
# Confirm: all tools work, no regressions
```

### Step 2 — Create a new vault with llama.cpp backend

**Prerequisites:**
- llama.cpp built at `/Users/jandahlke/dev/hagbards_stuff/_utils/llama.cpp`
- Models:
  - Chat: `Qwen_Qwen3.5-9B-Q8_0.gguf`
  - Embed: `nomic-embed-text-v1.5.Q8_0.gguf`

```bash
# Create fresh vault directory
mkdir /tmp/eris-llamacpp-test && cd /tmp/eris-llamacpp-test
eris chat

# During ignition:
#   Backend: llama.cpp
#   llama.cpp home: /Users/jandahlke/dev/hagbards_stuff/_utils/llama.cpp/build
#   Chat model: /path/to/Qwen_Qwen3.5-9B-Q8_0.gguf
#   Embed model: /path/to/nomic-embed-text-v1.5.Q8_0.gguf
#   Context window: 65536
#   GPU layers: 99 (or whatever fits)
```

**What to watch in the startup log:**
```
[startup] Compiled dynamic per-tool GBNF grammar for llama.cpp
    tool_count=<N>  typed_count=<N>  fallback_count=<N>  grammar_len=<N>
```
- `typed_count` should be close to `tool_count` (most/all tools compiled)
- `fallback_count` should be 0 or very small

### Step 3 — Live tool call test cases

Run these prompts in the llama.cpp vault and verify correct tool dispatch:

| # | Prompt | Expected tool | Validates |
|---|--------|---------------|-----------|
| 1 | "what time is it" | `clock:now` | Empty args `{}` |
| 2 | "read my identity file" | `vault:read` | Single required string arg |
| 3 | "write 'hello world' to test.md" | `vault:write` | Three required args + enum (`mode`) |
| 4 | "set a 5 minute timer called 'tea'" | `clock:timer` | Integer + string args |
| 5 | "remember that my favorite color is blue" | `memory:stage` | Mixed optional fields + array (tags) |
| 6 | "system health check" | `system:health` | Empty args, complex response |
| 7 | "search my vault for 'project'" | `vault:search` | String + optional fields |
| 8 | "list my agenda" | `agenda:list` | Simple args |
| 9 | "remind me in 30 minutes to stretch" | `agenda:remind_at` | All-optional fields |
| 10 | "list files in 10_Episodic" | `vault:list` | Single required string |

**Success criteria for each:**
- Model output is valid JSON (grammar enforced — no parse failures)
- Tool name is valid (from the registered set)
- Tool args match the schema (no Gatekeeper `SchemaViolation` errors)
- Tool executes successfully

**Failure indicators to watch for:**
- `GRAMMAR BUG` in logs → grammar is too restrictive, model can't produce valid output
- `SchemaViolation` errors → grammar allows something the schema rejects (field names, types)
- llama-server errors about grammar parse → GBNF syntax issue
- Model stuck / infinite generation → grammar has an unsatisfiable path

### Step 4 — Grammar stress test

```bash
# Multi-tool call in one turn
"what time is it, and also check system health"

# Tool with enum field
"write 'test content' to notes.md in append mode"

# Rapid consecutive turns
"read test.md" → "now search for 'hello'" → "set a timer for 2 minutes called 'break'"
```

### Step 5 — Benchmark comparison

If the operator has a benchmark setup:
- Compare grammar-constrained llama.cpp output vs Ollama for the same prompts
- Key metrics: JSON parse success rate, Gatekeeper validation pass rate, generation speed (tokens/sec)

---

## After testing succeeds: Phase 8 — Documentation

Follow `docs/LLAMACCP_GRAMMAR/9_PHASE8_DOCUMENTATION.md`:

1. **Create `docs/LLAMA_CPP_SETUP.md`** — operator-facing setup guide
   - Building llama.cpp (macOS Metal, Linux CUDA, CPU-only)
   - Obtaining GGUF models (the user's actual models: Qwen3.5-9B-Q8_0, nomic-embed-text-v1.5.Q8_0)
   - Config.toml reference with real values from the test vault
   - Running (managed vs external llama-server)
   - Switching backends (Ollama ↔ llama.cpp)
   - Troubleshooting the 7 common failure modes

2. **Update `docs/OPERATOR_MANUAL.md`** — add Backend Selection section with pointer to setup guide

3. **Update metaplan acceptance checkboxes** in `0_METAPLAN.md` and `8_PHASE7...md`
   - Two remaining checkboxes (live test): "llama-server accepts and enforces the generated grammar", "Tool calls from grammar-constrained output pass Gatekeeper validation"

---

## Hard rules reminder

- No `unwrap()`/`expect()` outside `#[test]`
- No `unsafe`
- No `println!` — use `tracing`
- No `Arc<Mutex<T>>` across async — use channels
- Tests that write to disk use `tempfile`
- **Do not run `git commit`** — operator commits only

---

## Key files for the next instance

| Area | Files |
|------|-------|
| Grammar compiler | `src/engine/grammar/schema_to_gbnf.rs` (new, 633 LOC) |
| Grammar assembly | `src/engine/grammar/envelope.rs` (`compile_fcp_envelope_grammar_dynamic`) |
| Grammar public API | `src/engine/grammar/mod.rs` |
| Session wiring | `src/executive/chat_session.rs` (lines 705–733) |
| Phase 8 spec | `docs/LLAMACCP_GRAMMAR/9_PHASE8_DOCUMENTATION.md` |
| Operator manual | `docs/OPERATOR_MANUAL.md` |
