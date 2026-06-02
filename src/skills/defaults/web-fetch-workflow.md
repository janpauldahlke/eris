---
id: web-fetch-workflow
title: Web fetch, find, and news — read receipts and respect the ledger
priority: conditional
triggers: web:fetch,web:find,web:search,news:today
---
Use this skill whenever you call a web tool. browser39 does not run site JavaScript; receipts tell you what actually happened.

## Full text vs receipt (do not confuse these)
- **`web:fetch` stores the full page body** under `20_Discourse/web/missions/<mission_id>/pages/<artifact_id>/` (markdown chunks + `links.json`).
- The tool JSON you see is a **receipt only**: `preview_head` is capped (~600 chars). It is **not** the whole article.
- **Never tell the user** that `web:fetch` “only returns a preview.” Say: full text is in the mission vault; use **`web:find`** to read it.
- **`chunk_count`** = how many vault chunks were stored (often `1` for a full article), not “preview only.”
- **`web:find`** returns **snippets** (per-match caps). One find is not the full 10k+ char body — run several finds with **terms that appear in the article** (headline words, names, places) to extract or summarize.

## Read the receipt first
After `web:fetch` or `news:today`, quote fields from the tool result JSON — do not guess.

1. **`receipt_summary`** — one line (`page_quality`, `consent_*`, `chunk_count`, `cached`, `artifact_id`; for find: `match_count`, optional `best_match_url`).
2. **`next_step_hint`** on fetch — authoritative next action (usually `web:find` on `artifact_id`).
3. **`page_quality`**: `ok` | `thin` | `likely_consent_or_js` | `likely_paywall` — respect before claiming you read an article.
4. **`fetch_budget_remaining`** — pages left in this mission; omit `fetch_budget` on fetch unless the user caps pages (config default applies; new missions cannot go below `default_fetch_budget`).
5. After **`web:find`**: `match_count`, **`best_match_url`** (prefer this for the next `web:fetch`), `matches[]` / **`link_matches[]`**, optional `suggest_stop`. Do not use `vault:read` on mission paths.

## Sites for depth testing
| Site | Use |
|------|-----|
| **taz.de** | Full-article depth tests (no paywall) — preferred for home→find→fetch→summarize |
| **zeit.de** | Headlines/teasers OK; many articles are **Z+ paywall** (`page_quality=likely_paywall`) — do not claim full article text |
| **kicker.de** etc. | Often JS/CMP walls — follow consent hints and cookie preload in docs |

## Multi-step flows (home → article → summarize)
- **Omit `fetch_budget`** unless the user explicitly caps pages.
- Keep the same **`mission_id`** from the first fetch receipt on follow-up fetches.
- Copy **`best_match_url`** from `web:find` JSON into the next `web:fetch` — never invent URLs from memory or snippet order alone.
- On a **homepage** (`chunk_count` often 1): rank headlines via `internal_links_sample` / `link_matches`; find queries should match **visible headline text** (e.g. “Mietenpolitik”), not unrelated topics.
- To **summarize an article**: `web:find` with words from the **article subject** (title, entities), not the user’s unrelated keywords — `match_count=0` means the query missed, not “fetch failed.”
- Run only the steps the user asked for; do not insert extra `web:find` queries between scripted steps.

## Tool choice
| User intent | Tool |
|-------------|------|
| Full URL | `web:fetch` |
| Headlines / “today’s news” / homepage digest | `news:today` |
| Search without a URL | `web:search` then **`web:find`** on the SERP `artifact_id` |
| Search inside an already-fetched page | `web:find` only |

## Same-host policy (`require_find_before_refetch`)
- After a successful `web:fetch` on a host, another `web:fetch` on that host may be **blocked** until you run `web:find` on the existing `artifact_id` — unless you pass the same `mission_id` from the receipt.
- After **`news:today`** completes, you may fetch that homepage host again without `web:find` first (ledger clears the host gate).
- A **second fetch of the exact same URL** returns a **cache hit** (`cached=true`) with the stored `artifact_id` — use a **different path** on the same host to fetch another article.

## Consent and JS walls
- `consent_attempted=true` + `consent_improved=false` + `page_quality=likely_consent_or_js` → static HTML had almost no body.
- `page_quality=likely_paywall` → subscription teaser only; try **taz.de** or another open article.
- Do not claim you read full article content when `preview_head`, find snippets, or paywall quality indicate otherwise.

## Args discipline
- `web:fetch`: `url`, optional `mission_note`, `mission_id`, optional `selector`, optional `explore_site` (config-gated) — omit `fetch_budget` unless the user caps fetches.
- `web:find`: `artifact_id` (UUID from receipt), `query`, optional `top_k`, `mission_id`.
- `web:search`: `query` only (plain text, not a URL).
