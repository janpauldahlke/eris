# Open-source & shipping roadmap (Apache 2.0) — July 2026

Companion to [10_DEEP_REVIEW_2026-07.md](./10_DEEP_REVIEW_2026-07.md). That file is *what's wrong inside*; this file is *how to get it into other people's hands*. Written as a second-order observation: it names the blind spots in the plan as stated, then gives the plan.

---

## 0. Second-order observations (read first)

**O1 — "Removing the copyright" is a category error.** You cannot and should not remove copyright; Apache 2.0 *requires* a copyright notice. Open-sourcing means: keep `Copyright (c) 2026 Jan Dahlke`, add the Apache 2.0 `LICENSE`, and delete the "All Rights Reserved" claim in `README.md:462–471`. Your copyright is the thing that gives the license force.

**O2 — You are conflating two different releases.** "Public this weekend" and "user-ready" are different products with different bars:

- *Public source* = someone can read, build, and judge the code. Bar: license, no secrets, honest README. **Achievable this weekend.**
- *User-ready* = someone who is not you gets to a working chat in <15 minutes without reading source. Bar: installers, first-run UX, engine defaults that don't require tuning. **Not achievable this weekend, and pretending otherwise burns the only first impression you get.**

Ship them as separate milestones (M0/M1 below). Tag M0 `v0.1.0-alpha` and say "alpha, macOS-first" out loud.

**O3 — Your CLA contradicts your stated goal.** `docs/CLA.md` + README §Copyright are built to *retain unilateral re-licensing and monetization*. That is a defensible strategy — but it is the opposite signal of "moving on to open source", and for a solo project with zero contributors it is solving a problem you don't have yet while creating friction for the contributors you want. Decide which project you're running:

| Model | Inbound | When it fits |
|---|---|---|
| **Plain Apache 2.0, inbound=outbound + DCO sign-off** (recommended) | Contributions land under Apache 2.0, `Signed-off-by` line | You want users and drive-by contributors; you keep copyright on your ~100% of the code anyway, which is de facto re-licensing power for years |
| Apache 2.0 + CLA | Contributors grant you re-licensing rights | You have concrete commercial plans (dual licensing, cloud offering) |

Recommendation: DCO now, delete or park `docs/CLA.md` and the CLA bot wiring (`signatures/`). You can introduce a CLA later *before* accepting large external contributions — you cannot un-scare people retroactively.

**O4 — The Docker instinct is wrong for your primary platform.** You develop on macOS. Docker on macOS has **no GPU/Metal passthrough** — a containerized `llama-server` runs CPU-only and will be 5–20× slower. Eris is a local-first, latency-sensitive TUI app whose whole value is fast local inference. So:

- **Do not** dockerize the eris binary or llama-server as the default install path.
- **Do** ship a `docker-compose.yml` for **Qdrant only** (it's the one dependency that's annoying to install natively and needs no GPU).
- **Do** offer a full-stack `docker-compose` (eris + llama-server-CUDA + qdrant) as the *Linux/NVIDIA server* path — that's where containers actually shine, and `peripherals.rs` process management can be bypassed there (containers replace the spawn/reap logic).

**O5 — The real onboarding cliff is not the binary, it's the models.** A new user needs: the eris binary, a `llama-server` binary, a chat GGUF (multi-GB), an embedding GGUF, and Qdrant. Your installer story lives or dies on automating the model download + `--ctx-size` defaults, not on packaging Rust. Budget most of M1 there.

**O6 — Your README is written for you, not for users.** 36k of architecture-diary is great material but it's the wrong front page. Users need: what is this (3 sentences), a screenshot/GIF of the TUI, quickstart (5 commands), hardware requirements table, license. Move the diary to `docs/`.

---

## 1. Engine decision: llama.cpp only for prod

**Recommendation: yes — llama.cpp is the sole supported production backend. Demote Ollama to "experimental/dev" status, don't delete it yet.**

Rationale, from the code:

1. **GBNF is your reliability story.** The whole protocol (one JSON envelope per turn, per-turn subset grammars, `GbnfSubsetCache`) only has teeth on llama.cpp. Ollama's `FormatType::Json` is soft — it produces exactly the parse-failure → Recover loop class you built 1000 lines of `json_envelope.rs` to compensate for.
2. **The dual backend is a maintenance tax with divergent semantics** (system-message merging, temperature defaults, `num_ctx` handling, trailing-prose recovery that is Ollama-only). Every orchestrator change is currently tested against two behavioral profiles.
3. **Self-sufficiency already exists:** `peripherals.rs` spawns a dedicated `llama-server --embedding` (line ~644), so embeddings do not require Ollama. Qdrant is backend-agnostic.
4. **Caveat that keeps Ollama alive short-term:** Ollama's model management (`ollama pull`) is a better *download UX* than hand-fetching GGUFs. Until your installer automates GGUF downloads (M1), Ollama remains the easiest on-ramp for some users. Mark it "works, unsupported, weaker JSON discipline" rather than ripping it out this weekend.

**Precondition:** the llama.cpp long-context P0 fixes from the deep review (§3: cap `n_predict`, reserve completion window, truncation-vs-protocol-fault classification) land **before** M1. Shipping llama.cpp-only *with* the truncation bug means new users hit your worst failure mode on day one.

---

## 2. Milestones

### M0 — "Public source" (this weekend)

Legal/hygiene, ~1 day of work. Nothing here touches code behavior.

| # | Task | Detail |
|---|------|--------|
| 1 | `LICENSE` | Apache 2.0 verbatim text, `Copyright 2026 Jan Dahlke` |
| 2 | `NOTICE` | One line: project name + copyright (Apache convention) |
| 3 | `Cargo.toml` | `license = "Apache-2.0"`, `repository`, `description` fields |
| 4 | README rewrite (top) | What/why/screenshot/quickstart/hardware/license; move the diary to `docs/`. Delete §Copyright "All Rights Reserved" + CLA strategy text |
| 5 | Park the CLA | Remove `docs/CLA.md` + `signatures/` or mark deprecated; add DCO note in `CONTRIBUTING.md` |
| 6 | **Full history secret scan** | Run `gitleaks detect --source . --log-opts="--all"` (and/or trufflehog). Manual scan found nothing (no `.env` in history, `vaults/` never tracked, Moltbook keys via env/file, Bearer strings are test fixtures) — but run the tool before flipping the switch. If anything surfaces: rotate the credential; only rewrite history if unavoidable |
| 7 | Untracked cruft | Add `.idea/` to `.gitignore` (currently 9 untracked `.idea` files); confirm `.DS_Store` handling; do **not** commit them |
| 8 | Honest CLI | From review P0: delete or stub-error `eris run` / `eris tool` (silent no-ops are embarrassing in public) — 30 min |
| 9 | `SECURITY.md` | Contact address + "local-first, no telemetry leaves the machine" statement |
| 10 | Tag | `v0.1.0-alpha`, GitHub release with "known limitations" section (be blunt: macOS-first, long-context JSON issue open, installers pending) |

Explicitly **not** in M0: installers, Docker, engine consolidation, any refactor.

### M1 — "Installable" (1–2 weeks after)

The goal: stranger → working chat in 15 minutes, macOS Apple Silicon first.

| # | Task | Detail |
|---|------|--------|
| 1 | **Long-context P0 fixes** | Deep-review §3 items 1–3. This is the top *user-perceived* quality issue |
| 2 | Release CI | Extend `.github/workflows/`: build `aarch64/x86_64-apple-darwin` (script exists: `scripts/build-release-targets.sh`) + `x86_64-unknown-linux-gnu` on ubuntu runner; attach to GitHub Releases |
| 3 | `install.sh` | curl-pipe installer: fetch binary, fetch/locate `llama-server` (brew `llama.cpp` or GitHub release), offer model download (one blessed chat GGUF + `nomic-embed` GGUF with checksums), write starter `config.toml` |
| 4 | Qdrant path | `docker-compose.qdrant.yml` + docs; keep native-binary instructions as alternative |
| 5 | llama.cpp default | Default `llm_backend = "llamacpp"` in generated config; Ollama documented as experimental |
| 6 | First-run UX | You already have `setup_welder/` + preflight — wire them into a single `eris setup` that validates binaries, ports, models, Qdrant, and prints one green/red table. Fix: preflight currently probes daemons for the stub commands |
| 7 | Wire `log_level` | Review P0 #4 — public users will file "why is my disk full of debug logs" otherwise |
| 8 | Homebrew tap | `brew install janpauldahlke/tap/eris` — cheap once release CI exists; defer if time-boxed |

### M2 — "User-ready / community-ready" (weeks 3–6)

- `docker-compose.full.yml` for Linux/CUDA servers (eris + llama-server + qdrant; document that peripherals-spawning is disabled in container mode).
- `CONTRIBUTING.md` (build, test matrix, the Absolute Laws from `.cursorrules` — they're a genuinely good contributor contract), issue/PR templates, `CHANGELOG.md` (keep-a-changelog).
- Config versioning/migration story (113-field `AppConfig` **will** change; users' TOMLs must not break silently — add a `config_version` key now, cheap).
- Docs restructure: `docs/HOW_TO/END_USER_README.md` and `OPERATOR_MANUAL.md` promoted to a docs site or wiki; architecture docs stay for contributors.
- Demo assets: 30–60s TUI GIF (asciinema/vhs) — this does more for adoption than any doc.
- Drift-prevention P1 items from the deep review (tool inventory CI test, data-driven gatekeeper allowlists) — before contributors add tools and trip on the 5–8 edit points.

### M3 — later / optional

- Windows target (MSVC triple is in `release-targets.txt` comments; llama-server exists for Windows — real work is peripherals process management).
- Ollama backend removal decision (revisit once installer-driven GGUF flow is proven).
- Orchestrator decoupling P2 (deep review §8) — do it *before* accepting large external PRs to `orchestrator/`, otherwise you'll be reviewing god-method diffs from strangers.
- CLA reconsideration only if a concrete commercial plan appears (see O3).

---

## 3. Privacy & trust posture (write it down once, early)

Eris's differentiator is local-first. Make the promise explicit and auditable:

- **Statement in README + SECURITY.md:** no telemetry leaves the machine; `.fcp/telemetry/` is local logs only; vault/memories never transmitted except to *user-configured* endpoints (Google Workspace, Discord, Moltbook, Open-Meteo, web fetch).
- **Enumerate the outbound surfaces** (that list above *is* the audit — it's short and you already gate web via allowlist/consent/ledger).
- Ship `config.toml` defaults with all network-touching integrations **off** (Discord, Moltbook, GWS, web) — opt-in, not opt-out. First impressions of an "agent" that immediately wants tokens for three services are bad.

---

## 4. Weekend checklist (M0 condensed, in order)

```text
[ ] gitleaks detect --log-opts="--all"        # abort-gate: rotate anything found
[x] LICENSE + NOTICE + Cargo.toml license field
[x] README: rewritten user-facing; full reference moved to docs/REFERENCE.md;
    "All Rights Reserved" + CLA prose deleted
[x] CLA removed (docs/CLA.md, .github/workflows/cla.yml, signatures/);
    CONTRIBUTING.md with DCO added
[x] .gitignore += .idea/
[x] `eris run` + `eris tool` now return honest not-implemented errors
[x] SECURITY.md with privacy statement
[x] Codecov removed from CI (clippy + sharded tests remain)
[ ] Record TUI demo GIF (placeholder marked in README)
[ ] GitHub: repo settings (issues on, discussions optional); GitHub Sponsors profile
[ ] Tag v0.1.0-alpha, release notes with honest "known limitations"
```

Everything else is M1+. If the weekend runs short, items 1–3 alone are a legitimate "public" — the rest can follow Monday.
