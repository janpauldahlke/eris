# browser39 integration (MVP)

## Decision

**Use subprocess + JSONL (`browser39 batch`), not an in-process Rust dependency.**

| Approach | Verdict |
|----------|---------|
| **A. Subprocess** (JSONL per fetch) | **Chosen** — process isolation; no V8/deno_core in the eris link graph |
| **B. In-process crate** | **Not viable on crates.io today** — `autolib = false`; only the `browser39` binary is published |

Implementation: `WebFetcher` / `Browser39Fetcher` under `src/tools/web/fetcher.rs`. Each fetch uses a **dedicated session directory** `.fcp/browser39/sessions/{artifact_id}/`, runs `browser39 batch` with `--no-persist`, and reads `results.jsonl`. Vault pages are cached under `20_Discourse/web/missions/{mission_id}/pages/{artifact_id}/`.

Phase 2 (planned): per-host `browser39 watch` sessions, optional disk persistence, consent profiles — see the web post-MVP hardening plan.

## Operator install

```bash
cargo install browser39 --locked
# or a release binary from https://github.com/alejandroqh/browser39/releases
browser39 --version
```

Vault template (ignition): `{vault}/.fcp/browser39/config.toml` — `[search].engine`, timeouts; eris merges **`user_agent`** from `AppConfig` on chat bootstrap.

Allowlist: `{vault}/.fcp/web_allowlist.toml` (toggle with `[web].allowlist_enabled`).

## MVP protocol (per fetch)

1. Create `.fcp/browser39/sessions/{artifact_id}/`.
2. Write one `fetch` line to `commands.jsonl` (`show_selectors_first: false`, `include_links: true`, pagination via `offset`).
3. Run `browser39 batch --no-persist --config <vault-config>`.
4. Parse the first `results.jsonl` line; paginate until `WebBudget` cap.

Full schema: [jsonl-protocol.md](https://github.com/alejandroqh/browser39/blob/main/docs/jsonl-protocol.md).

## Config (`[web]` in vault `.fcp/config.toml`)

| Key | Default | Notes |
|-----|---------|--------|
| `allowlist_enabled` | `true` | Enforce `.fcp/web_allowlist.toml` |
| `search_enabled` | `true` | Register `web:search` |
| `require_find_before_refetch` | `true` | Same-host ad-hoc refetch needs `web:find` first; **continuing an existing `mission_id` is exempt** (e.g. `news:today` deep fetches) |
| `max_web_tool_calls_per_turn` | `2` | Turn cap for web tools (orchestrator); consider `4` for heavy news sessions |
| `max_fetches_per_user_turn` | `2` | Ledger fetch cap per user turn |
| `persist_ledger` | `false` | Optional `.fcp/web_session.json` across restarts |

## Limits (honest expectations)

- **No JS execution** in browser39 — SPAs and heavy client-rendered sites may return thin markdown; receipts include `page_quality` (`thin`, `likely_consent_or_js`) and hints.
- Cookie-banner lines are stripped in `sanitize_markdown_noise`; consent walls may still need Phase 2 host profiles or manual `[[cookies]]` in browser39 config.
- Not a Playwright/CDP replacement.

## Testing

- Unit tests use `MockWebFetcher` (default CI).
- Optional live smoke: `BROWSER39_INTEGRATION=1` with `#[ignore]` tests when wired.
