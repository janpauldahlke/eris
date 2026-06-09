# Hybrid-GPU GNOME session drop (`hybrid-gpu-gnome-session-drop`)

**Also known as (old misname):** “test-full OOM” — see [Why the old name was wrong](#why-the-old-name-was-wrong).

**Status:** understood + workarounds in place · **Priority:** monitor  
**Issue label:** `hybrid-gpu-gnome-session-drop`  
**Host:** FUCKUP — Ubuntu 24.04, kernel 6.17, AMD Raphael iGPU (displays) + RTX 4080 SUPER (`prime-select on-demand`), Xorg session.

---

## One-sentence summary

> **`cargo test-full` does not fail — the GNOME graphical session sometimes dies while it runs.** Same tests pass via `cargo test` (~26s) and via detached `test-full` (all 47 batches, exit 0).

---

## Do we need local `cargo test-full`?

| Tool | Needed locally? | Why it exists |
|------|-----------------|---------------|
| `cargo test` | **Yes** — daily dev | Runs all 682 unit tests in one process (~26s here) |
| `cargo test-full` | **No** — optional | Local mirror of CI’s 47-batch shard list |
| GitHub CI test matrix | **Yes** — on push | Parallel batches on `ubuntu-latest` (memory-safe cloud runners) |
| GitHub CI + Codecov | **Yes** — on push | Same batch list, `cargo llvm-cov` per shard — **does not require local test-full** |

**Codecov is not why you run test-full at home.** CI uploads coverage on push. Local `test-full` was added so developers could replay CI’s batch plan before pushing — originally for **laptop RAM OOM**, not for Codecov.

**Practical workflow (FUCKUP / hybrid-GPU desktops):**

```bash
# Daily
cargo test

# Pre-push (let CI be the full gate)
git push

# Optional: local full-suite parity without killing GNOME
./scripts/test-full-detached.sh
tail -f target/test-full.log

# Avoid in Cursor/GNOME terminal (Heisenbug — session roulette)
cargo test-full
```

---

## Named issue: what is it?

**`hybrid-gpu-gnome-session-drop`** — intermittent Heisenbug where:

1. A **long job** runs inside the **GNOME graphical session** (Cursor terminal, GNOME Terminal).
2. After roughly **5–15 minutes**, the **user systemd session** activates `exit.target`.
3. User lands on **GDM login** — not a test failure, not a kernel RAM OOM on this host.
4. Journal shows **`amdgpu` LTTPR / async-flip DRM errors** on `0000:11:00.0` at teardown.
5. Timing **correlates** with `cargo test-full` (many batches, long wall time) but **the same tests pass** when the session does not die.

**What it is NOT:**

- Not a failing Rust test (log has no `FAILED` at the cliff; `cargo test` passes all 682).
- Not classic RAM OOM on FUCKUP (61 GB RAM, ~57 GB free; no `systemd-oomd … Killed` lines).
- Not terminal scroll/redraw alone (quiet mode still dropped; output went only to log file).
- Not required for Codecov (CI handles coverage).

**When it got worse:** ~1 month before 2026-06-09 investigation — user moved display cables to **AMD iGPU**, set **iGPU primary in BIOS** (to free NVIDIA VRAM for compute). Cliff correlated with that change.

### Corroboration: r/gnome hybrid GPU thread (2026)

Thread: [Hat jemand Probleme mit der Hybrid NVIDIA GPU?](https://www.reddit.com/r/gnome/comments/1s5tldz/any_having_trouble_hybrid_nvidia_gpu/) (Debian 13, GNOME 48, Intel+NVIDIA laptop).

| Their report | Our report (FUCKUP) |
|--------------|---------------------|
| HDMI on **NVIDIA**; after unplug, dGPU **stays awake** | Cables on **AMD iGPU**; NVIDIA still loaded (`on-demand`) |
| Fix: **logout/login** or restart GNOME shell | Symptom: **forced logout** (`exit.target`) mid long job |
| Display topology change → broken GPU power/routing | Display topology change (cable/BIOS) → session instability |
| “Hybrid on GNOME is fiddly” | Same — not a Rust/cargo bug |

**Same family of bug, different trigger:** they fight “NVIDIA won’t sleep after unplug”; we fight “session dies under sustained load.” Both are **GNOME + hybrid GPU + display routing** — the Reddit OP’s logout workaround is literally the failure mode we hit involuntarily.

Comment thread notes the **irreconcilable tension**: force everything onto iGPU (env vars, `MESA_VK_DEVICE_SELECT`, etc.) and **external/NVIDIA display paths break**; use NVIDIA for HDMI and **iGPU desktop gets weird**. We chose iGPU-for-displays + NVIDIA-for-compute — same tightrope.

**We are not alone.** Do not treat this as an Eris-specific defect.

---

## Why the old name was wrong

The doc was titled “SOFTEN_TEST_FULL_OOM” assuming a **RAM-constrained laptop**: ~400 MB test binary × many batches → OOM killer.

On FUCKUP (2026-06-09 evidence):

| Check | Result |
|-------|--------|
| `free -h` | ~57 GB available |
| `journalctl \| grep oomd.*Killed` | **empty** this boot |
| `executive::peripherals::` peak RSS | **~39 MB** (`/usr/bin/time -v`) |
| Terminal `vte-spawn` scope peak | **~1.4 GB** (whole job tree — not 61 GB) |
| Journal at drop | `exit.target` + `gnome-shell: X connection broken` + amdgpu LTTPR |

**Rust forum “cargo killed on Ubuntu”** reports are often **`systemd-oomd`** killing `vte-spawn` during compile — a **different mechanism** with the same symptom (terminal vanishes). Worth checking `journalctl` for `oomd` if symptoms change.

---

## Timeline / log evidence (`target/test-full.log`)

| Phase | Batches | Outcome |
|-------|---------|---------|
| Early runs | 41 | `=== all batches passed ===` (interactive) |
| + `tools::media::` | 42 | Dies at `[9/42] START: executive::` — no `DONE` |
| Executive split | 47 | Dies at `[13/47] START: executive::peripherals::` — no `FAILED` |
| Quiet mode (GUI) | 47 | Same cliff ~batch 13 — disproved “terminal redraw only” |
| `cargo test` (GUI) | 682 in one process | **Passed ~26s** — twice |
| `./scripts/test-full-detached.sh` (cron) | 47 | **`=== all batches passed ===`**, exit 0 |

**Misleading correlation:** cliff often lands on `executive::` / `peripherals::` because that is ~batch 9–13 after ~5–10 minutes — **duration + batch index**, not proof those tests are broken.

---

## Architecture (why three ways to run tests)

```
┌─────────────────────────────────────────────────────────────┐
│  cargo test          │  one process, all 682 tests, ~26s   │
│  (local daily)       │  fine on FUCKUP if session stays up │
├─────────────────────────────────────────────────────────────┤
│  cargo test-full     │  47 subprocess batches, ~10–15 min   │
│  (eris-test-full)    │  mirrors CI shard list sequentially │
│                      │  risky in GNOME on hybrid-GPU box    │
├─────────────────────────────────────────────────────────────┤
│  CI matrix (GitHub)  │  same 47 filters, PARALLEL VMs       │
│  + Codecov shards    │  no local desktop involved             │
└─────────────────────────────────────────────────────────────┘
```

| File | Role |
|------|------|
| `src/bin/eris_test_full.rs` | Local batch runner: warm build once, direct test binary per batch, quiet by default, auto-resume from log |
| `scripts/test-full-detached.sh` | One-shot **cron** job — survives GNOME logout (runs outside user graphical session) |
| `.github/workflows/ci.yml` | `test` + `coverage` matrix — **source of truth** for batch list |
| `.cargo/config.toml` | `test-full` (quiet), `test-full-verbose`, `t` aliases |

---

## Runner improvements (done 2026-06-09)

- [x] Split `executive::` into 6 sub-batches (47 total)
- [x] CI matrix synced (including `tools::media::`)
- [x] Resume / `ERIS_TEST_FROM` / `--from` from `target/test-full.log`
- [x] Direct prebuilt test binary (skip per-batch `cargo test` wrapper)
- [x] Quiet mode default (`test-full` → log file only)
- [x] `scripts/test-full-detached.sh` (cron + flock)
- [x] `MALLOC_ARENA_MAX=2`, inter-batch pause

---

## If it happens again — diagnosis checklist

```bash
# 1. Tests or session?
tail -30 target/test-full.log          # look for FAILED vs cut mid-batch

# 2. OOM vs GPU/session?
journalctl -b 0 | grep -iE 'oomd.*Killed|LTTPR|exit\.target' | tail -20

# 3. Resume or detached rerun
ERIS_TEST_FROM=N cargo test-full       # or
./scripts/test-full-detached.sh
```

| Log says | Meaning |
|----------|---------|
| `*** FAILED:` / `test result: FAILED` | Real test bug — fix the test |
| Last line `[N/47] START:` no `DONE` | Session/process killed — **this issue** |
| `oomd … Killed vte-spawn` | Classic Ubuntu oomd — different fix path |
| `LTTPR` + `exit.target` | **hybrid-gpu-gnome-session-drop** |

---

## Safe local commands

```bash
# Default dev (preferred)
cargo test -- --test-threads=1

# Targeted
cargo test --bin eris executive::router:: -- --test-threads=1

# Full suite without GNOME roulette
./scripts/test-full-detached.sh
```

---

## Open / optional (do not block dev)

- [ ] `cargo test-full` completes interactively in GNOME without drop (may require display-stack fix outside repo)
- [ ] Rename this file to `HYBRID_GPU_GNOME_SESSION_DROP.md` once link rot is acceptable
- [ ] Lighten `relay_submit_then_system_inject_orders_after_tool` if profiling shows benefit (not proven on FUCKUP)

---

## Copy-paste prompt for a future Cursor instance

```
Read docs/TODO/SOFTEN_TEST_FULL_OOM.md (issue: hybrid-gpu-gnome-session-drop).

Context: GNOME session drops to login during long interactive cargo test-full on
FUCKUP (AMD iGPU displays + NVIDIA, kernel 6.17). NOT RAM OOM. cargo test passes.
Detached test-full passed all 47 batches.

Do NOT assume test failure. Check target/test-full.log and journalctl for
exit.target / LTTPR / oomd.

Local workflow: cargo test daily; ./scripts/test-full-detached.sh for full suite;
CI+Codecov on push. Do not run interactive test-full without user OK.
```
