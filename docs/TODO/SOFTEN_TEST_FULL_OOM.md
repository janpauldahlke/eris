# Soften `cargo test-full` — OOM / session drops on laptops

**Status:** open · **Priority:** high · **Owner:** next Cursor instance  
**User pain:** ~3 weeks of session drops when running the full suite on a RAM-constrained Ubuntu machine (gemma12B vault + Ollama often running).

---

## Problem (what we know)

`cargo test-full` (alias → `eris-test-full`) runs **42 subprocess batches** with `--test-threads=1`. Each batch is a fresh process so RSS *should* reset — but the **user's session still dies** repeatedly at the same point.

### Log evidence (`target/test-full.log`)

| Run | Batches | Outcome |
|-----|---------|---------|
| 1 | 41 | `=== all batches passed ===` |
| 2 | 42 (added `tools::media::`) | Dies at **`[9/42] START: executive::`** — no `DONE`, no test failure |
| 3+ | 42 | Same: restarts from batch 1, dies again at batch 9 |

After a drop: no stuck `cargo test` / `llama-server`; only normal `ollama serve` remains. **Likely OOM killer or IDE/session memory pressure**, not a failing assertion.

### Batch order (batch 9 is the cliff)

```
 8  memory::          ← 54 tests, passes
 9  executive::        ← 39 tests, NEVER finishes (drop)
10  benchmark::
```

---

## Current architecture

| File | Role |
|------|------|
| `src/bin/eris_test_full.rs` | Batch list, warm build once, spawn `cargo test --bin eris <filter> -- --test-threads=1` per batch, append to `target/test-full.log` |
| `.cargo/config.toml` | `test-full` / `t` aliases; `RUST_TEST_THREADS=1`; `build.jobs = 2` |
| `.github/workflows/ci.yml` | Same batch list in matrix (parallel CI shards; **missing `tools::media::`** until synced) |
| `Cargo.toml` `[profile.test]` | `debug = 0`, `opt-level = 1` — already tuned for smaller test binary |

### Test binary size (important)

On disk, `target/debug/deps/eris-*` test executables are **~350–420 MB** each. Loading that binary + heap for heavy tests can push **1–2+ GB RSS** per batch even with `--test-threads=1`. Eight batches in a row + Cursor + Ollama + browser ≈ cliff at batch 9.

### `executive::` tests (39 total)

| Module | Count | Notes |
|--------|-------|-------|
| `executive::cli::` | 13 | Lightweight CLI parse tests |
| `executive::peripherals::` | 9 | Mock TCP for llama health; spawns `sleep 300` children for shutdown tests |
| `executive::router::` | 3 | **`relay_submit_then_system_inject_orders_after_tool` builds full `Orchestrator` + `Gatekeeper` + `SystemHealthTool`** — heaviest |
| `executive::identity_md::` | 3 | Tempdir + async FS |
| `executive::setup_welder::` | 2 | Light |
| `executive::error::` | 2 | Light |

`executive::chat_session.rs` has **no unit tests** (integration lives elsewhere).

---

## Hypotheses (ranked)

1. **Cumulative memory pressure** — 8× (cargo wrapper + 400 MB binary mmap + test heap) while IDE/Ollama hold RAM; batch 9 tips over.
2. **Single heavy test** — `router::relay_submit_then_system_inject_orders_after_tool` allocates a large orchestrator graph; peak during that one test kills the session.
3. **Cargo per-batch overhead** — each batch re-invokes `cargo test` (~60s just to `--list`); metadata + jobserver + duplicate work, not just test RSS.
4. **Not a logic failure** — no `FAILED` line in log; if it were a panic we'd usually see output before drop.

---

## Recommended fixes (implement in this order)

### 1. Split `executive::` into sub-batches (quick win)

In `eris_test_full.rs` **and** `.github/workflows/ci.yml` (keep lists in sync):

```rust
// Replace single "executive::" with:
"executive::cli::",
"executive::error::",
"executive::identity_md::",
"executive::setup_welder::",
"executive::peripherals::",
"executive::router::",   // isolate heaviest batch
```

Also add missing CI batch: `"tools::media::"`.

**Why:** Isolates the heavy router test; if drop moves to `executive::router::`, you've confirmed the culprit; smaller failure surface for resume.

### 2. Add `--from` / auto-resume to `eris_test_full`

After a drop, user shouldn't replay batches 1–8 every time.

- Parse env `ERIS_TEST_FROM=9` or CLI `--from 9`
- On start, scan `target/test-full.log` for last `[N/42] DONE ok:` and offer to continue at N+1
- Log line: `=== resuming from batch 9 ===`

### 3. Invoke prebuilt test binary directly (skip cargo per batch)

After `cargo build --bin eris --tests`:

```bash
# Find newest test exe (hash changes on rebuild)
exe=$(ls -t target/debug/deps/eris-*.exe target/debug/deps/eris-[0-9a-f]* 2>/dev/null | head -1)
"$exe" executive::router:: --test-threads=1
```

Or use `cargo test --no-run` then run the artifact path from `target/debug/deps/`. **Avoids cargo metadata + wrapper RSS** on every batch.

### 4. Profile before guessing (one command, user-approved)

```bash
/usr/bin/time -v cargo test --bin eris executive::router:: -- --test-threads=1 --nocapture 2>&1 | tee target/executive-router-mem.log
```

Check `Maximum resident set size`. Run **only** this batch — do **not** run full `test-full` until softened.

Per-test isolation:

```bash
cargo test --bin eris executive::router::tests::relay_submit_then_system_inject_orders_after_tool -- --test-threads=1
```

### 5. Lighten heavy tests (if router batch is the peak)

In `src/executive/router.rs` `relay_submit_then_system_inject_orders_after_tool`:

- Ensure `tempfile::TempDir` is the only FS touch; no vault seeding beyond minimal `00_Invariants`
- Avoid registering tools that pull optional deps if not needed for ordering proof
- Consider splitting into slimmer mock (stub `SystemHealthTool` that doesn't call real health backends)
- Do **not** spawn real Qdrant/Ollama/llama-server in unit tests

In `peripherals` shutdown tests: `sleep 300` stubs are fine; verify children are always reaped (already uses `shutdown_async`).

### 6. Optional: split `memory::` (54 tests)

If drops persist after executive split, shard similarly:

```
memory::types::
memory::ephemeral::
memory::semantic::
memory::prefetch::
memory::turn_end::
```

### 7. Runner ergonomics

- **`MALLOC_ARENA_MAX=2`** in batch subprocess env (reduces glibc arena fragmentation on Linux)
- **Short pause** between batches (`tokio::time::sleep` 1s or std::thread::sleep) — lets kernel reclaim pages
- **Document preflight:** suggest closing chat/Ollama models before `test-full` on laptops (`ollama ps`, stop heavy models)

### 8. Do NOT do by default

- Run full `cargo test-full` in agent loops without user OK
- Add `unwrap()`/`expect()` in production code while fixing tests
- Commit unless user asks

---

## Safe verification commands (use these instead of full suite)

```bash
# Targeted (media catalog work in progress)
cargo test --bin eris blob_store media::card tools::media:: tools::vision::validate -- --test-threads=1

# Single executive submodule after split
cargo test --bin eris executive::router:: -- --test-threads=1

# Resume-friendly (after --from is implemented)
ERIS_TEST_FROM=9 cargo test-full
```

---

## Parallel feature work (context only — do not block OOM fix)

Uncommitted **vision / media catalog** work exists on branch `feature/improve-ui`:

- `99_USER_UPLOADED/` content-addressed blobs, `40_MEDIA/{hash}/media.json`
- Tools: `vision:see`, `vision:display`, `media:catalog`, `media:meta`
- `tools::media::` batch already added to `eris_test_full.rs` (42 batches)

Plan file: `.cursor/plans/vision_media_catalog_e30603e0.plan.md`

Finish OOM softening **first**; then continue feature polish / e2e on `vaults/gemma12B/`.

---

## Acceptance criteria

- [ ] `cargo test-full` completes on user's machine without session drop (or user confirms batch 9+ pass with resume)
- [ ] `executive::` split + CI matrix synced (including `tools::media::`)
- [ ] Resume/`--from` works from `target/test-full.log`
- [ ] README or comment in `eris_test_full.rs` documents safe partial verify + preflight
- [ ] (Optional) Direct test-binary invocation reduces batch startup time measurably

---

## Copy-paste prompt for a fresh Cursor instance

```
Read docs/TODO/SOFTEN_TEST_FULL_OOM.md and target/test-full.log.

Goal: stop session drops during cargo test-full on a RAM-limited laptop.

Do NOT run full test-full until changes are in place.

1. Split executive:: into sub-batches in src/bin/eris_test_full.rs and .github/workflows/ci.yml (add tools::media:: to CI too).
2. Add --from / ERIS_TEST_FROM resume to eris_test_full.
3. Optionally invoke the prebuilt test binary directly after warm build.
4. Profile executive::router:: with /usr/bin/time -v (single batch only, ask user before running).
5. If router batch peaks high, slim relay_submit_then_system_inject_orders_after_tool.

Verify with targeted cargo test commands only. No git commit unless I ask.
```
