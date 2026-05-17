# browser39 integration (Phase 0 spike)

## Decision (MVP)

**Use subprocess + JSONL (`browser39 watch`), not an in-process Rust dependency.**

| Approach | Verdict |
|----------|---------|
| **A. Subprocess** (JSONL per fetch) | **Chosen** — matches upstream agent examples; process isolation; no V8/deno_core in the eris link graph |
| **B. In-process crate** | **Not viable on crates.io today** — see below |

Implementation: `WebFetcher` trait with `Browser39Fetcher` spawning an isolated `watch` session per fetch under `{vault}/.fcp/browser39/sessions/{artifact_id}/`, `--no-persist`, vault config via `BROWSER39_CONFIG` / `--config`. See the browser39 web MVP plan for ledger, mission cache, and anti-crawl policy.

## Spike evidence (browser39 1.7.1)

Surveyed [crates.io/browser39](https://crates.io/crates/browser39) (1.7.1), registry sources, [docs.rs](https://docs.rs/browser39/latest/browser39/), and upstream [jsonl-protocol.md](https://github.com/alejandroqh/browser39/blob/main/docs/jsonl-protocol.md).

1. **No published library crate.** `Cargo.toml` sets `autolib = false` and declares only `[[bin]] name = "browser39"`. Internal modules (`core`, `service::BrowserService`) exist in source but are **not** exposed as `use browser39::…` for downstream crates.
2. **Documented agent contract is transport-level:** JSONL `watch` / `batch`, or MCP `browser39 mcp`. The canonical Rust example [`examples/browser39_tools.rs`](https://github.com/alejandroqh/browser39/blob/main/examples/browser39_tools.rs) uses `std::process::Command`, `commands.jsonl` / `results.jsonl`, and a long-lived watch child — not a library API.
3. **Adding `browser39` as an eris dependency would not help integration:** Cargo would still build the binary only; eris could not call `BrowserService` without forking browser39 or upstream enabling `autolib`. It would also pull **deno_core / V8** into the dependency graph with no usable API.

Revisit in-process integration only if a future browser39 release publishes a stable library surface explicitly intended for embedders.

## Operator install (subprocess mode)

Install the CLI on `PATH` (pick one):

```bash
cargo install browser39 --locked
```

Or download a release binary from [browser39 releases](https://github.com/alejandroqh/browser39/releases).

Verify:

```bash
browser39 --version
```

Vault-side template (ignition): `{vault}/.fcp/browser39/config.toml` — commented timeouts/viewport; chat bootstrap merges **`user_agent`** from `AppConfig` only.

Optional CI / local smoke (post-implementation): `BROWSER39_INTEGRATION=1` with `#[ignore]` integration test; default CI uses `MockWebFetcher` only.

## Protocol sketch (per-fetch isolation)

Per MVP plan, each `web:fetch` uses a **dedicated** watch session (not the example’s singleton long-lived client):

1. Create session dir `.fcp/browser39/sessions/{artifact_id}/`.
2. `touch` `commands.jsonl` and `results.jsonl`.
3. Spawn: `browser39 watch --config <vault-config> --no-persist` (exact flags per `browser39 --help` when wiring `client.rs`).
4. Append one `fetch` action (URL, `options.max_tokens`, `selector`, pagination `offset`) to `commands.jsonl`; read results from `results.jsonl`.
5. Paginate until `WebBudget` byte/chunk cap; terminate child.

Full action schema: upstream [jsonl-protocol.md](https://github.com/alejandroqh/browser39/blob/main/docs/jsonl-protocol.md).
