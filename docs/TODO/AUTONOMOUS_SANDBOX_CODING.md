# Autonomous Sandbox Coding

**Status:** Design / not started
**Owner discussion:** operator + agent (eris), 2026-08-24
**Goal (one line):** Give the model the ability to write and run code completely
autonomously inside an air-gapped Docker sandbox, structure its own work so it
does not lose the thread in a small context window, and eventually author its
own tools — without ever recompiling the Rust binary to add a tool.

**The emerging shape** — three general tool families + one general loop:

- **`plan:*`** — durable task ledger (working memory; replaces the polluting
  `agenda_self` self-nudge role; general to *all* tasks, not just coding).
- **`sandbox:*`** — act (write/run/iterate on code in an air-gapped container).
- **dynamic tools** — grow (the model authors its own tools as data).
- **focus-loop mode** — one parameterized loop state, tagged by task type
  (coding, research, triage, …), that ties the families together.

---

## 0. The core tension, resolved

Eris is a single statically-linked Rust binary. Tools are concrete Rust types
implementing the async `Tool` trait (`src/tools/traits.rs`), hand-registered at
startup into `Gatekeeper`'s `HashMap<String, Arc<dyn Tool>>`. On llama.cpp the
tool-call JSON is hard-constrained by a GBNF grammar. So the natural worry is:
*how do we add dynamic tools to a thing that "compiles hard all the time"?*

**Key discovery that makes this feasible:** the harness **already compiles GBNF
grammars from JSON schemas at runtime, per turn** (`GbnfSubsetCache::get_or_compile_subset`
in `src/orchestrator/core/llama_gbnf_subset.rs`, plus `src/engine/grammar/schema_to_gbnf.rs`).
Every hop it builds a fresh grammar for exactly the tools offered that turn. So
the *schema* side of a tool is already dynamic data. Only the *executor* (the
Rust code that runs the tool) is compiled in.

**Resolution — separate definition from execution:**

- **Tool definition** = pure data: `name`, `description`, JSON schema, entrypoint
  (script + interpreter). Created / edited / versioned at runtime, **zero recompiles**.
- **Tool execution** = today a bespoke Rust `impl Tool`. For dynamic tools it
  becomes **one generic Rust executor** that takes a manifest + args, drops into
  the sandbox, runs the entrypoint, returns stdout.

You compile the *harness* (the fixed meta-tool layer) once. After that, the agent
authoring a new tool is just: write a file in the container + write a manifest.
The "compiles hard all the time" friction never touches the agent's tool-creation
loop. This mirrors how **skills** already work (markdown data, not code) and how
Cursor's own MCP dynamic tools work.

### The GBNF question, resolved: it constrains the *call*, not the *code*

The grammar governs one thing only: the JSON the model *emits* in the FCP
envelope — `{thought, status, message_to_user, tool_calls:[{name, args}]}`. That
is the tool's **interface** (name + argument shape). GBNF never sees the tool's
**implementation** — the Python in `/scratch` is arbitrary unconstrained code on
the far side of the boundary. So "do his tools respect GBNF?" reduces to "does
his tool declare a JSON schema?" — and the manifest requires one, fed through the
*same* `schema_to_gbnf` machinery native tools use.

**Strictness is a spectrum, not on/off** (see Phase 3 for the recommended path):

- **A — full schema→GBNF:** args grammar-constrained to the exact schema. Max
  strictness; requires `schema_to_gbnf` to cover every schema feature used, and
  complex schemas bloat the grammar / slow decoding.
- **B — generic-object args (recommended first):** grammar still enforces the
  envelope shape **and constrains the tool name to the registered set** (no
  hallucinated tools), but `args` may be *any valid JSON object*; the gatekeeper
  validates args against the schema with `jsonschema` at execution (as it already
  does) and the existing schema-recovery path handles mistakes.
- **C — hybrid:** start a tool in B, graduate it to A once its schema is proven.

We do **not** drop GBNF — we *relax it on the args only, for the dynamic
namespace*. Name + envelope stay hard-constrained.

---

## Repo structure — the `leif-erikson` crate

> **The substrate crate is `leif-erikson`, and it lives in its own repo.** Its
> from-scratch bootstrap is speced separately in `LEIF_ERIKSON_HANDOVER.md` (the
> handover for the new instance). This section only covers how *eris* consumes it.


**Motivation:** eris is currently a *single crate* (~277 files, one compilation
unit), so every source edit recompiles the whole thing. External deps like
bollard are already compiled once and cached — editing eris never rebuilds them —
so the real win of a split is **isolating the sandbox as its own compilation
unit**: editing eris core no longer recompiles sandbox code, and the sandbox
crate stays cached when untouched. It also forces a clean API boundary. (Precedent:
the Google Workspace split via the published `gws-builder` crate — same spirit,
though that one is a build-dependency codegen tool; `leif-erikson` is a runtime dep.)

**Hosting (DECIDED): separate git repo, consumed as a cargo dependency.**
Motivated by agent cognitive load — an agent working on the sandbox clones a small
focused repo; an agent on eris core sees one `Cargo.toml` line, not the sandbox
internals. Feasible precisely because the crate is pure substrate (no eris types).
- **Separate repo `leif-erikson`**, depended on via git (or published) version.
- **Local co-dev** via `[patch]` / path override pointing at a local clone — keeps
  iteration easy without vendoring the source into eris.
- **Avoid git submodules** — a well-known footgun for agents (detached HEAD,
  forgotten `--recursive`, confusing status). Not worth the pain.
- (A single-repo cargo **workspace member** at `crates/leif-erikson` remains the
  fallback if cross-repo coordination proves annoying, but it doesn't give the
  clone-only-what-you-need isolation the operator wants.)
- **Pure substrate, no eris types.** The crate must not know about eris's `Tool`
  trait or `FcpError`. It exposes:
  - `SandboxRunner` trait + `BollardSandbox` impl
  - plain types: `ExecRequest`, `ExecOutput`, `FileOp`, `SandboxConfig`
  - its own `SandboxError` (thiserror)
  - the devpi proxy network setup
  - It mirrors the laws: no `unsafe`, no `unwrap`/`expect` in prod, async, error via `?`.
- **Eris keeps the thin adapter layer** at `src/tools/sandbox/`: `impl Tool` calls
  the crate and maps `SandboxError → FcpError`; gatekeeper/descriptor/skill wiring
  stays in eris.
- **Feature-gate** the dependency (`sandbox` feature) so builds without it never
  pull bollard.
- **`plan:*` and focus-loop stay in eris** — control-plane concerns, not substrate.
  Only the container/exec/network layer leaves.

### Config flow — eris → leif-erikson (dependency inversion)

The crate must **not** read config files or env; it receives a plain config
struct at construction, so eris stays the single source of config truth and the
crate stays reusable/testable.

- Crate exposes its own plain types:
  ```rust
  pub struct SandboxConfig { pub network: NetworkMode, pub image: String,
      pub mem_limit: u64, pub cpu_quota: u64, pub pids_limit: i64,
      pub exec_timeout: Duration, pub max_concurrent_execs: usize,
      pub scratch_size_mb: u64, pub max_output_bytes: usize }
  pub enum NetworkMode { Off, PypiProxy { /* proxy settings */ } }
  ```
- eris parses the `[sandbox]` TOML (figment) into its own settings, then maps
  into `leif_erikson::SandboxConfig` and calls `SandboxManager::new(cfg)`.
- **Two independent toggles, by design:** the compile-time `sandbox` cargo feature
  gates the dependency entirely; the runtime `enabled` + `network` fields gate
  behavior. No "reach back into eris" coupling.

---

## Network model — devpi pull-through egress

**Goal:** "allow only Python packages, nothing else." Achieved without an internet
hole by routing egress through a package proxy, keeping all container hardening.

- **Topology:** the sandbox attaches to an **`internal: true`** docker network
  (no gateway to host/internet). A small **devpi proxy container** sits on both
  that internal net *and* a normal net with outbound access — so the sandbox's
  *only* reachable peer is the proxy.
- **uv points at the proxy** (`UV_INDEX_URL` / `PIP_INDEX_URL`); there is no other
  route out. Arbitrary hosts don't resolve and have no path.
- **The proxy only speaks the package-index protocol** (devpi pull-through cache):
  the sandbox can ask "give me package X" but cannot make arbitrary outbound
  requests. Bonus: it caches (fast repeat installs) and logs every package pulled.
- **Managed like Qdrant** — another peripheral in `peripherals.rs` (lazy-started
  with the sandbox, reaped on shutdown), or set up by the `leif-erikson` crate.

**Residual risk (honest):** a malicious/typo-squatted package can enter and run
code on install/import — but with egress limited to the proxy it **cannot phone
home or exfiltrate**; it's boxed by the hardening (burns capped CPU at worst), and
the proxy log shows exactly what was pulled. Risk shifts from "network
exfiltration" (closed) to "supply-chain + container escape" (escape mitigated by
non-root + cap-drop + seccomp + read-only rootfs). Materially smaller surface than
open network, slightly larger than pure air-gap — acceptable given the sacrificial
host and the `network=off` fallback.

**Modes:** `sandbox.network = off` (fully offline, uv against a pre-seeded cache) or
`pypi-proxy` (this). Default `off`.

---

## 1. Agreed decisions

| Decision | Choice | Rationale |
|---|---|---|
| Container driver | **bollard** (pure-Rust async Docker API) | Clean async exec + resource limits; satisfies the `unsafe` ban. Added *alongside* the existing docker-CLI Qdrant sidecar, not replacing it. |
| Tool language | **Python** | Fastest for a small model to write correctly; rich stdlib. |
| Isolation posture | **Egress-restricted** (devpi proxy only) + full container hardening | Read-only rootfs, non-root, cap-drop, seccomp, resource limits stay. The *only* network route is a devpi pull-through proxy; no other egress. Fully-offline mode still selectable. See §Network model. |
| Host risk context | **Sacrificial box** — dedicated OS + SSD, wipeable | Host-compromise paranoia dials down; hardening is mainly for reproducibility + not corrupting the agent's own work + preventing vault exfiltration. |
| Network model | **devpi pull-through proxy** | Sandbox reaches *only* a local package proxy on an `internal` docker net; the proxy is the sole peer with outbound access, speaks only the package-index protocol, caches + logs every pull. |
| Repo structure | **`leif-erikson` in its own git repo**, cargo dependency (feature-gated) | Pure substrate crate; eris keeps thin `impl Tool` adapters. Chosen for agent cognitive-load isolation (clone only what you need). No submodules. See §Repo structure. |
| Task ledger | **Refactor `agenda` → `plan:*`** | `agenda` collides with `clock:alarm`/`calendar`; scheduling moves there, task-memory moves to `plan:*`. See §2.2. |
| Model exclusivity | **Modal focus-loop** (one model) | Coding suspends chat; operator watches thinking + can **hold** (cancel loop, keep container/`/scratch`/`plan`, resumable). Second "coder" model is Phase 4+. See §2.4. |
| Container start | **Lazy** (first sandbox use) | Not every chat needs the sandbox. |
| `AgentState`s with sandbox | **Chat + Idle** | Idle enables background/unattended scripts ("process this while you're away") — with timeouts + caps enforced. |
| Base image | **Pinned by digest**, batteries-included | A `python:slim` update must never break the agent's scripts. Pre-seed compute libs (numpy, pandas, polars, scipy). |
| Package manager | **uv** (not pip) | Single static binary → trivial to bake in; far faster; installs via the devpi proxy (or offline cache in `network=off` mode). |
| Dynamic tool manifests (Phase 3) | **`10_Topology/tools/`** | It's infrastructure → belongs in Topology. |
| `/scratch` persistence | **Session-lived** (tmpfs; dies with container) | Persists across turns within a session; a named Docker volume is an opt-in later for cross-restart durability. |
| Config toggles | **Layered** — `sandbox.enabled`, `sandbox.network` (`off`\|`pypi-proxy`), `focus_loop.enabled` | Sandbox without the autonomous loop; offline vs proxy egress; all default off/most-restrictive. |

---

## Phase 1 — Prove the exec loop (build first)

**Goal:** the model can write a small Python program into an air-gapped
container, run it, and read the result — through the normal gatekeeper/GBNF
path, **off by default**.

### 1.1 The `leif-erikson` crate (external repo, substrate) + eris adapters
- `SandboxManager` backed by **bollard**, run as an **actor** owned by the
  orchestrator or reached via `mpsc` (Law 4: no `Arc<Mutex>` shared across
  threads). All exec is async (Law 3: never block the tokio runtime).
- A `SandboxRunner` **trait** at this boundary (traits-at-boundaries rule) so the
  tools depend on the trait and unit tests can mock it without Docker.
- The substrate lives in the **`leif-erikson` crate** (see §Repo structure); the
  `impl Tool` adapters live in eris at `src/tools/sandbox/`.
- Container spec (hardening posture):
  - `NetworkMode`: either **`none`** (`network=off`) or attached **only** to an
    `internal: true` docker net whose sole other peer is the devpi proxy
    (`network=pypi-proxy`). No route to host/internet either way. See §Network model.
  - `ReadonlyRootfs: true`, writable **tmpfs** at `/scratch` (size-capped)
  - non-root user, `CapDrop: ALL`, `no-new-privileges`, seccomp default
  - hard limits: memory, CPU quota, `PidsLimit`, and a **per-exec wall-clock
    timeout** (kill the exec, not the container); a **max-concurrent-exec** cap
    matters especially in Idle so an unattended loop can't churn.
  - **no host bind-mounts** — the vault is never mounted into the container
- Lifecycle: **lazy-start** on first sandbox tool use; **one container per
  session**, reaped on session end mirroring the `ManagedProcess` reap pattern in
  `src/executive/peripherals.rs`.

### 1.2 `/scratch` semantics (session working directory)
- `/scratch` **persists across turns and execs within a session** — write a file
  in turn 1, read it in turn 12. It is the agent's working directory, not a
  per-exec temp.
- tmpfs is RAM-backed and **dies with the container** (session end). Cross-restart
  durability is a deliberate later opt-in via a **named Docker volume** at
  `/scratch` (still not the vault, still air-gapped).
- **DECIDED: session-scoped `/scratch` is enough for now.** Operator "hold" cancels
  the loop without reaping the container (see §2.4), so interruption doesn't lose
  work. Cross-restart durability deferred.
- **Separate concern (don't conflate):** managing a **uv project/venv** (`uv venv`,
  `uv add`, `pyproject.toml`) is *project structure*, not *persistence*. In Phase 1
  the agent drives uv through `sandbox:exec` (+ the thin `sandbox:install` wrapper);
  a dedicated uv-project tool family is only worth adding later if exec-driven uv
  proves clumsy.

### 1.3 The export seam (how work escapes the sandbox)
`/scratch` is air-gapped from the vault by design, so finished work needs a
deliberate bridge — otherwise a tool/artifact the agent writes can never become a
real vault note. Phase 1: `sandbox:read_file` → `vault:write`. Consider a
dedicated `sandbox:export { scratch_path, vault_path }` later. This is the seam
between "played in the sandbox" and "committed to memory."

### 1.4 Config (`src/config.rs`)
New `[sandbox]` section:
- `enabled` (default **false**)
- `network` = `off` | `pypi-proxy` (default **`off`**)
- `image` (pinned Python image by digest, batteries-included)
- `mem_limit`, `cpu_quota`, `pids_limit`
- `exec_timeout_secs`, `max_concurrent_execs`
- `scratch_size_mb`
- `max_output_bytes`

### 1.4a Base image contents (pre-installed, offline-useful) — DECIDED
Ships in the pinned image so the agent can just `import` (works in `network=off`
too). Curated for **offline compute / data / scripting** — **no HTTP-client or
web-server libs** (external network is blocked, so they'd be useless). Python's
**stdlib already covers** json, csv, sqlite3, pathlib, subprocess, re, datetime,
dataclasses, argparse — so the extras below are the number-crunching + parsing +
testing belt.

- **Data / compute:** `numpy`, `pandas`, `polars`, `scipy`
- **Parsing / serialization:** `pyyaml`, `orjson`, `lxml`, `beautifulsoup4`,
  `python-dateutil`
- **Modeling / validation:** `pydantic`
- **Templating / codegen (offline):** `jinja2` — generate files/code from templates
- **Plotting (writes image files, `Agg` backend, no display):** `matplotlib` —
  pairs naturally with eris's media catalog (render a chart → catalog the PNG)
- **Testing / quality:** `pytest`, `ruff`, `black`, `mypy`
- **CLI / ergonomics:** `rich`, `tqdm`, `click`

Optional if wanted: `sqlalchemy` (sqlite is stdlib), `sympy` (symbolic math).
Anything beyond this comes via `sandbox:install` (proxy) or a pre-seeded cache
(`network=off`).

### 1.5 The fixed meta-tools (`src/tools/sandbox/`)
Compiled `impl Tool`s with static `&'static str` names/descriptions and static
schemas → they land in the startup GBNF envelope automatically.

**Author / edit (small models need cheap iteration, not overwrite-only):**
- `sandbox:write_file { path, contents }` — create / overwrite
- `sandbox:append_file { path, contents }` — cheap incremental writes
- `sandbox:edit_file { path, old, new }` — **anchored string replace** (unique
  `old` → `new`). Deliberately **not** line-range edits: small models miscount
  lines and numbers drift after every edit; anchored replace is far more robust.

**Navigate (keep big files out of a small context):**
- `sandbox:read_file { path, range? }` — optional byte/line range
- `sandbox:list { path? }`
- `sandbox:grep { pattern, path? }` — locate code without dumping whole files

**Run:**
- `sandbox:exec { command, args?, stdin? }` — run in `/scratch`, return
  `{ stdout, stderr, exit_code }`, truncated to `max_output_bytes`

**Package management with uv (mode-dependent):**
- `sandbox:install { package }` — installs via **uv**:
  - `network=pypi-proxy`: `uv pip install <pkg>` resolving through the devpi proxy
    (only packages, only via the proxy; see §Network model). Self-serve, anytime.
  - `network=off`: `uv pip install --offline <pkg>` against a pre-seeded uv cache
    (`UV_CACHE_DIR`) or `--no-index --find-links=/wheels` against a wheelhouse —
    curated pantry only, no network.
- For the batteries-included libs (numpy/pandas/polars/scipy) the agent just
  `import`s — they're already present, no install step.
- Rationale: uv is a single static binary (easy to bake into a pinned image),
  much faster than pip, first-class offline/locked resolution.

### 1.6 Wiring into existing machinery
- **Register** in `src/executive/chat_session.rs` gated by `config.sandbox.enabled`.
- **Descriptors** in `src/tools/specs.rs`: add TOML entries — satisfies the boot
  invariant `assert_covers_registered_tools` and gives `ToolRouter` its embeddings
  automatically.
- **Gatekeeper allowlist** (`state_allows_tool` in `src/tools/gatekeeper.rs`):
  allow in `Chat` + `Idle` only when enabled; never in `Reflect`.
- **Per-turn cap**: reuse the web-tool-cap pattern in
  `src/orchestrator/core/tool_dispatch.rs` to bound exec calls per turn (holds in
  Idle too).

### 1.7 The "dynamic super skill" (Phase-1 flavor)
- New embedded default `src/skills/defaults/sandbox-coding.md` (+ seeded to the
  vault via `src/skills/seed.rs` / `defaults.rs`) teaching the
  write→edit→run→read loop, scratch semantics, the export seam, and the air-gap
  constraints.
- Wire it via `suggested_skills` on `sandbox:exec` so it JIT-injects when offered.

### 1.8 Safety + telemetry
- `tracing` events for every exec (command, exit code, duration, truncation flag).
- Output truncation, timeout enforcement, error taxonomy through `FcpError`.

### 1.9 Tests
- Unit tests against the mocked `SandboxRunner` (no Docker; `tempfile` if touching fs).
- One `#[ignore]`d integration test that actually spins bollard (manual / docker CI lane).

### Phase 1 deliverable checklist
- [ ] depend on `leif-erikson` (external repo) as a **feature-gated cargo dependency** (see `LEIF_ERIKSON_HANDOVER.md` for the crate itself)
- [ ] `impl Tool` adapters in eris wrapping the crate's `SandboxRunner`; map `SandboxError → FcpError`
- [ ] devpi proxy peripheral + `internal` docker net wiring (`network=pypi-proxy`); `network=off` fallback
- [ ] `/scratch` session persistence semantics + reap on session end
- [ ] `[sandbox]` config section (incl. `network`), default disabled / `off`
- [ ] meta-tools: write / append / edit (anchored) / read (range) / list / grep / exec (+ install)
- [ ] export seam (`read_file` → `vault:write`, optional `sandbox:export`)
- [ ] descriptors in `specs.rs`
- [ ] gatekeeper allowlist (Chat + Idle) + per-turn cap + max-concurrent-exec
- [ ] `sandbox-coding.md` skill (embedded + seeded + `suggested_skills` wiring)
- [ ] telemetry + truncation + timeout
- [ ] mocked unit tests + one ignored bollard integration test

---

## Phase 2 — `plan:*` ledger + focus-loop mode (make multi-step work survivable)

**Why this matters:** coding ≠ writing one file. With a small context window the
killer failure is **goal amnesia** — the model writes step 1, exec output floods
the window, and by step 4 it has forgotten *why* it started. The fix is a
well-known pattern: an **external, durable task ledger the harness re-injects into
the prompt every turn**, so forgetting becomes structurally impossible. This is
general — it helps research, triage, memory synthesis — coding is just where it's
most acute.

### 2.1 `plan:*` — a compiled Rust tool family (control-plane state)
Must be **Rust-compiled, not a sandbox script**: the harness has to persist it
reliably and render it into context, so it cannot be agent-authored data in the
container. Rough shape:
- `plan:set { goal }` — the north star
- `plan:add_step`, `plan:complete_step`, `plan:block_step`
- `plan:note` — a scratch pinboard for findings ("bug was in the parser")

**The magic isn't the tools — it's the rendering.** The current plan block is
**force-injected into the prompt every hop** (like `Identity.md` and skills are),
so it survives context churn. Persistent working memory that lives *outside* the
token window.

### 2.2 Refactor `agenda` → `plan` (DECIDED)
Operator decision: **refactor `agenda` and let `plan:*` take over the task role.**
Rationale: `agenda`'s time-triggered prompts frequently **collide with `clock:alarm`
and `calendar`** — three overlapping ways to say "later." Split by responsibility:
- ***When*** (time-triggered) → lives in **`clock:*` / `calendar:*`**. `agenda`'s
  scheduling role folds into those; stop duplicating it.
- ***What / why*** (task state, "don't lose the thread") → **`plan:*`**, with
  explicit `complete`/`clear` instead of the accreting `agenda_self` pollution.
- Net: `agenda:*` is retired as a family; existing self-nudge/reminder behavior
  migrates to `plan:*` (task memory) + `clock`/`calendar` (scheduling). Watch for
  callers in routing/skills/descriptors during the refactor.

### 2.3 focus-loop mode (one parameterized state, many task types)
Rather than proliferating hard-coded states (the gatekeeper's `state_allows_tool`
is a `match` on the `AgentState` enum, so every new state = editing that match
everywhere), add **one parameterized "focus loop" state with a task-*type* tag**
(coding, research, triage, …). The task type:
- selects which tool families are offered (coding unlocks `sandbox:*`),
- selects which skill is JIT-injected,
- drives the loop's termination budget (step count / wall-clock),
- pins the `plan:*` block + last-exec-result into context each hop,
- surfaces progress to the operator via `SessionEvent`,
- has a hard **escape hatch**: budget exhausted → break out and report, never spin
  forever.

The `plan:*` ledger is the shared spine under every task type; `sandbox:*` is the
coding flavor's extra family.

### 2.4 Model exclusivity + operator control (DECIDED)
We have **one model**, so it cannot chat and code at once. The focus-loop is
therefore **modal/exclusive**:
- Entering the loop **suspends normal chat** until it finishes or is held; the
  operator watches the model's **thinking stream** (already surfaced) for insight.
- **Operator "hold"/stop is the key control** (answers the "be able to hold him"
  requirement). Hold **cancels the loop** (via the existing `CancellationToken`)
  **but does NOT reap the container** — `/scratch` **and** the `plan:*` ledger
  survive, so he **resumes exactly where he left off**. Only *session end* reaps
  the container.
- This is why session-scoped `/scratch` is sufficient (see §Config: `/scratch`):
  interruption ≠ loss.

### Future (Phase 4+) — dedicated coder model / delegation
Escaping modality means running a **second model** as a scoped "coder" the main
eris orchestrator dispatches sub-tasks to (subagent pattern). Resolves the
"coder would miss the `plan` tool" worry: **the plan stays with the orchestrator**;
the coder is a stateless-ish executor given a bounded task and reporting results
back. Powerful but non-trivial (second llama-server/ollama instance, result
protocol, budget) — captured here, not built now.

### Phase 2 deliverable checklist
- [ ] `plan:*` compiled tool family + durable store
- [ ] always-on plan block rendering in context (gated to focus-loop / when non-empty)
- [ ] `focus_loop.enabled` config toggle (default off)
- [ ] parameterized focus-loop state + task-type tag → tool offering + skill + budget
- [ ] **refactor `agenda` → `plan` (task role) + `clock`/`calendar` (scheduling)**
- [ ] modal exclusivity: suspend chat during loop; thinking stream visible
- [ ] operator hold/stop = cancel loop, **preserve container + `/scratch` + `plan`**, resumable
- [ ] termination budget + escape hatch + progress `SessionEvent`s

---

## Phase 3 — Agent authors its own tools ("citizen, not user")

- **Manifest format**: `10_Topology/tools/<name>/manifest.toml` + `entrypoint.py`
  — name, description, JSON schema, interpreter, entrypoint.
- `sandbox:define_tool` meta-tool writes the manifest + script.
- A **generic `DynamicTool`** (`impl Tool`) that delegates to the `SandboxRunner`,
  passing args as JSON on stdin.
- **GBNF strictness: ship Option B first** (generic-object args + `jsonschema`
  validation at the gatekeeper), graduate mature tools to Option A (full
  schema→GBNF). Name + envelope stay hard-constrained throughout (see §0).
- **Hot-reload** into the gatekeeper `HashMap` + recompile the per-turn subset
  grammar. **Cache-key nuance:** `GbnfSubsetCache` is keyed by the sorted
  tool-name set — a redefinable dynamic tool must add a **hash/generation of the
  dynamic schema set** to the key, or a redefined tool serves a stale grammar.
- **Auto-descriptors** from manifests (relax `assert_covers_registered_tools` for
  the dynamic namespace).
- **`Tool` trait decision**: `name()`/`description()` are `&'static str`
  (`src/tools/traits.rs:10-11`). Two options:
  - `Box::leak` the manifest strings — safe, bounded by number of dynamic tools,
    pragmatic; leave the ~59 native tools untouched. **(leaning this)**
  - Widen the trait to `Cow<str>` / `&str` — mechanical but touches every tool.
- **Schema sanitizer** (mandatory): validate agent-authored JSON schema **before**
  it feeds `schema_to_gbnf`. Also **bound complexity** (max depth, max properties,
  no pathological regex) — this is a *performance* guard (grammar bloat / decode
  slowdown), not only a safety one.

---

## Risk ledger (the parts that actually matter)

- **Blast radius** — mitigated by container hardening + resource limits +
  off-by-default, on a sacrificial/wipeable host. Egress is closed except the
  devpi proxy (or fully off).
- **Supply-chain (via `network=pypi-proxy`)** — a hostile package can run code in
  the sandbox but cannot phone home (egress = proxy only); contained by
  non-root + cap-drop + seccomp + read-only rootfs; proxy logs every pull. Prefer
  `network=off` when self-serve installs aren't needed.
- **Unattended autonomy (Idle execs)** — the real leap: the heartbeat/agenda loop
  can trigger execs with no human present. Per-exec timeout + max-concurrent-exec
  + per-turn cap must all hold in Idle.
- **Grammar integrity** (Phase 3) — schema sanitizer + complexity bounds are
  mandatory before dynamic schemas reach GBNF; cache key must include schema
  generation.
- **Runtime hygiene** — bollard async / no runtime blocking, actor not mutex,
  `FcpError` everywhere, no `unwrap`/`expect`/`unsafe` (`.cursorrules`).

---

## Resolved decisions (this round)

1. **`plan:*` vs `agenda`** — ✅ **Refactor `agenda` → `plan:*`** (task memory);
   scheduling folds into `clock`/`calendar`. See §2.2.
2. **Base image contents** — ✅ curated Python web-dev + data list pinned into the
   image; rest via `sandbox:install`. See §1.4a.
3. **`leif-erikson` crate** — ✅ **own git repo, cargo dependency** (agent
   cognitive-load isolation); local dev via `[patch]`/path; **no submodules**.
   See §Repo structure.
4. **Cross-restart `/scratch`** — ✅ **session-scoped is enough now**; hold ≠ loss
   (operator hold keeps the container). uv project/venv management is a *separate*
   concern, driven via `sandbox:exec` for now. See §1.2, §2.4.
5. **Operator control** — ✅ the key requirement is **hold/stop**; focus-loop is
   **modal** (chat suspended, thinking visible). Second "coder" model deferred to
   Phase 4+. See §2.4.

## Still open (non-blocking)

- **Exact `pyproject`/base-image pin** — final digest + whether to also ship a uv
  cache for `network=off`.
- **`agenda` migration mechanics** — how to migrate existing persisted
  `agenda.json` tasks into the new `plan` store without data loss.
- **Second-model delegation protocol** (Phase 4+) — result format, budget,
  which backend (llama-server vs ollama) hosts the coder.
