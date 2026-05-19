# browser39 integration (MVP + consent)

## Decision

**Use subprocess + JSONL (`browser39 batch`), not an in-process Rust dependency.**

Implementation: `WebFetcher` / `Browser39Fetcher` under `src/tools/web/fetcher.rs`. Mission pages live under `20_Discourse/web/missions/{mission_id}/pages/{artifact_id}/`.

## Tool receipts (LLM-facing)

`web:fetch` returns compact JSON. Read these fields first:

| Field | Meaning |
|-------|---------|
| `receipt_summary` | One-line status (`page_quality`, `consent_*`, `chunk_count`, `artifact_id`) |
| `page_quality` | `ok`, `thin`, `likely_consent_or_js`, or `likely_paywall` |
| `consent_attempted` / `consent_improved` | Always present (booleans) |
| `next_step_hint` | What to do next (authoritative) |
| `artifact_id` / `mission_id` | For `web:find` and vault paths |
| `preview_head` | Short preview only (full text in vault chunks) |
| `internal_link_count` + `internal_links_sample` | Up to 5 sample links; full list in `links.json` |

`news:today` returns `receipt_summary`, `headline_count`, `deep_fetch_urls[]`, plus `headlines` / `deep_articles`.

**Skill:** `10_Topology/skills/web-fetch-workflow.md` (seeded on ignition) is injected when web tools are routed.

## Consent helper (Phase 2)

When `[web].consent_helper_enabled = true` (default), thin fetches trigger deterministic CMP handling:

1. `fetch` URL in a **host session** (`.fcp/browser39/sessions/hosts/{host}/`).
2. If markdown chars &lt; `thin_page_char_threshold`, try each `accept_link_text` from `.fcp/browser39/consent_profiles.toml` via browser39 **`click` by link text**.
3. Re-fetch the same URL in the same session (cookies kept when `persist_browser39_sessions` or consent is on).

Telemetry: `web.consent.thin_page`, `web.consent.accept_attempt`, `web.consent.click_failed`, `web.consent.refetch_ok` / `refetch_no_improvement`.

**Limits:** No JS execution — SPAs and iframe CMPs often need manual cookies. Set `use_legacy_batch = true` to disable host sessions and consent.

### Cookie preload (kicker and similar)

browser39 cannot click JS-rendered consent buttons. Workaround:

1. In Firefox/Chrome, open the site and accept cookies manually.
2. Export cookies for the domain (browser extension or devtools).
3. Add to `{vault}/.fcp/browser39/config.toml`:

```toml
[[cookies]]
domain = ".kicker.de"
name = "example_consent"
value = "..."
path = "/"
```

4. Re-run `web:fetch` — host session picks up preloaded cookies.

### Config (top-level + `[web]`)

| Key | Default | Notes |
|-----|---------|--------|
| `web_fetch_chunk_num_ctx_ratio` | `0.9` | Persisted mission chunk size = `num_ctx` × ratio (ceiling); optional `web_fetch_chunk_chars` override |
| `web_fetch_max_bytes` | `20480` in code default | Total markdown retained per fetch (before chunk split); raise in vault TOML for long articles |
| `consent_helper_enabled` | `true` | Thin-page accept attempts |
| `persist_browser39_sessions` | `false` | Disk cookies under `sessions/hosts/` (auto-on when consent enabled) |
| `thin_page_char_threshold` | `300` | Below this → try consent |
| `consent_max_attempts` | `2` | Max accept labels per fetch |
| `use_legacy_batch` | `false` | Per-artifact batch, no consent |
| `require_find_before_refetch` | `true` | Run `web:find` before second fetch on same host |
| `allowlist_enabled` | `true` | Set `false` only for local experiments |
| `persist_ledger` | `false` | `true` keeps `.fcp/web_session.json` across restarts |
| `explore_site_enabled` | `false` | Keep off unless you need cross-host BFS |

### Operator files

- `{vault}/.fcp/browser39/consent_profiles.toml` — seeded on ignition
- `{vault}/.fcp/browser39/config.toml` — optional `[[cookies]]` preload
- `{vault}/.fcp/web_allowlist.toml` — host patterns when allowlist enabled

### Ledger notes

- After **`news:today`** succeeds, the homepage host is cleared from `hosts_pending_find` so a follow-up `web:fetch` on that host is allowed.
- Continuing a mission: pass `mission_id` from the receipt on further fetches to skip the refetch gate.
- Duplicate URL: second `web:fetch` of the same normalized URL returns a cached receipt without a new browser hit.

## Operator install

```bash
cargo install browser39 --locked
browser39 --version
```

Chat startup runs `browser39 --version` when `[web] require_browser39 = true` (default) and seeds `.fcp/browser39/` plus `web_allowlist.toml` if missing. Override the binary with `BROWSER39_BIN`. Set `require_browser39 = false` only for local dev without the CLI.

## Testing

- Unit tests: mock only (default CI).
- Live: `BROWSER39_INTEGRATION=1` with `#[ignore]`.
