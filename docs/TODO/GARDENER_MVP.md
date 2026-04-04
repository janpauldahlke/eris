Wiki tool — single canonical plan (wiki:summary + Gardener)

This file is the only canonical plan. It is detail-rich for Phase 1 (immediate implementation) and Phase 2 (future Autonomous Gardener / Zettelkasten). A shorter duplicate plan file, if present, is deprecated and points here.

Scope rule: Implement Phase 1 in the current eris effort. Phase 2 is specified here for alignment but ships as a separate feature set (or multiple PRs).

Phase 1 vs Phase 2 at a glance

Dimension

Phase 1 — implement now

Phase 2 — future epic

Goal

Reliable oracle: English Wikipedia REST page/summary by article title → structured JSON to LLM/user

Living slip-box: synthesized zettels in 20_Semantic/ linking external concepts to your episodic work + optional embeddings

Owner

One Tool + ApiHttpClient + ApiProfile

Orchestrator / Idle “Gardener” task + existing tools (wiki:summary, vault:read, vault:write, optional memory:)

Vault I/O

None (read-only HTTP)

Reads episodic + semantic tree; writes new markdown notes

Qdrant

No automatic use

After vault file exists: optional memory:stage / commit per existing promote policy — never inside wiki:summary

Wikimedia

Profile User-Agent, encoded path

Reuses Phase 1 tool; same etiquette and rate discipline

Failure mode if merged wrong

—

Tool-level auto-save of extract = dumb cache, not a zettelkasten

Hard rule: wiki:summary must not call vault:write, memory:stage, or any “promote on success” side effect. Phase 2 composes tools from a higher layer.

Part A — Phase 1 (full specification)

A.1 Naming and routing

Tool name: wiki:summary — Wikipedia-specific, implies short summary. Avoid generic knowledge:lookup (collides with memory:query and web:fetch in embedding/JIT routing).

Clock/agenda lesson: tools that sound alike get misrouted. Use orthogonal axes:

Source: encyclopedia vs URL (web:fetch) vs vault/semantic memory (memory:query, vault tools).

Query shape: article title vs pasted URL vs “my notes.”

User phrasing: “What is …?”, “Who was …?”, “What does Wikipedia say about …?” — typically no URL, no “search my vault.”

flowchart LR
subgraph query [Query shape]
topic[Topic or article title]
url[Explicit URL]
vault[My notes / vault]
end
subgraph tools [Tool]
wiki[wiki:summary]
web[web:fetch]
mem[memory:query]
end
topic --> wiki
url --> web
vault --> mem

A.2 Boundaries (descriptor + Tool::description)

In scope

Out of scope

English Wikipedia REST page/summary by title; extract, optional description

Arbitrary URLs → web:fetch

High-level “what is / who is”

Private notes → memory:query, vault tools

Full HTML article; other wikis/languages until specified

One tight **Tool::description**: English Wikipedia lead summary by title; \*\*not for arbitrary URLs or vault search.

src/tools/specs.rs: when_not_to_use must name **web:fetch** and memory/vault tools. routing_hints: encyclopedia, wikipedia, who was, what is — \*\*not “fetch this link.”

src/orchestrator/tool_router.rs: enrich_for_routing line consistent with those hints.

A.3 HTTP, URL, Wikimedia

Endpoint: https://en.wikipedia.org/api/rest_v1/page/summary/{title}.

Encoding: Percent-encode the title path segment; trim; reject empty title → FcpError::SchemaViolation.

User-Agent: Set on the Wikipedia ApiProfile headers, e.g. User-Agent: Eris-Agent/1.0 (Local autonomous system). Wikimedia returns 403 for bot-like anonymous requests without a descriptive agent.

Profile id: e.g. wikipedia_page_summary colocated with tool code (pattern: src/tools/weather/open_meteo.rs PROFILE constants).

A.4 JSON envelope (LLM-facing)

Mirror weather-style stable JSON (not raw API-only):

source: e.g. "english_wikipedia" (disambiguate from vault / web:fetch).

title (resolved page title), extract, optional description.

canonical_url or equivalent from API content_urls when present.

One honesty line (in description or envelope): summaries may be incomplete or dated — avoid repeating in every descriptor example.

A.5 Codebase checklist (docs/ADDING_A_TOOL.md)

New module src/tools/wiki/ + pub mod wiki in src/tools/mod.rs.

Default API in src/config.rs merged with default_open_meteo_apis(); update default config test (apis count + key).

Register in src/executive/router.rs: add api_http.clone() in the chain so weather + wiki each hold Arc<ApiHttpClient>.

src/tools/gatekeeper.rs: allow in Chat / Reflect / Idle like weather; extend test_policy_covers_all_current_tools.

map_api_err-style mapping: api_client / network / parse → tool-scoped FcpError::ToolFault messages.

Tests: wiremock (path + User-Agent), unit tests for encoding + minimal JSON parse; follow src/tools/weather/mod.rs.

A.6 Phase 1 anti-patterns

Auto-persist or memory:stage inside wiki:summary.

Descriptor bloat that overlaps web:fetch / memory:query.

Raw Wikipedia paste into vault as “Phase 2” — that belongs in the Gardener as synthesis, not in the HTTP tool.

Part B — Phase 2 (full specification — future implementation)

Phase 2 is the Autonomous Gardener (working name): a background cognitive loop that runs when the system is Idle (or a dedicated “deep idle” mode), maintains connective tissue between the outside world and your vault, and never turns wiki:summary into a write path.

B.1 Thesis — encyclopedia vs Zettelkasten (Luhmann)

Bad outcome — caching: If every wiki:summary blindly wrote extract to 20_Semantic/foo.md, you get a local Wikipedia mirror. Embeddings would retrieve generic text with no contextual gravity relative to your projects.

Good outcome — zettels: Luhmann did not photocopy books; he wrote how ideas related to his research. 20_Semantic holds synthesized notes: a minimal attributed definition (grounded via the oracle) plus Contextual gravity — explicit links to your episodic log, names, decisions, tags, and other vault paths.

Implication: Phase 2 always includes an LLM synthesis step between oracle output and vault:write. The oracle reduces factual hallucination in the definition line; synthesis is still model-generated and must be user-auditable.

B.2 Target behavior — end-to-end narrative

Illustrative story (design target, not current code):

Episodic review: Gardener reads today’s material under a vault convention e.g. 10_Episodic/ (daily log, session notes). Example signal: “Hagbard and I wired the Actor Model using Rust MPSC for the clock alarm.”

Concept extraction / gap detection: From episodic text, derive candidate concepts (named entities, technical terms) or use a structured list. For each candidate, check whether 20_Semantic/ already contains a satisfactory note (by slug filename, frontmatter id, or small index file).

Knowledge gap: No existing 20_Semantic/actor_model.md (or equivalent).

Oracle query: Invoke **wiki:summary** with title e.g. "Actor model" — \*\*structured JSON only, no disk write.

Synthesis prompt (Gardener-internal): Inputs: episodic excerpt(s), oracle envelope (definition + canonical URL), optional links to related vault paths. Output: markdown body following a fixed zettel template (see B.4).

Persist: **vault:write** to e.g. 20_Semantic/actor_model.md (or Inbox/wiki/ first — see B.6). Content is \*\*not raw extract alone.

Optional embedding: If policy says semantic brain should index this note, run existing memory commit flow after the file exists — same rules as any other vault note.

flowchart TD
subgraph idle [Idle / Gardener scheduler]
T[Trigger: idle window / schedule]
T --> Epi[Read 10_Episodic]
Epi --> Cands[Candidate concepts]
Cands --> Gap[Gap check 20_Semantic]
Gap -->|missing| Wiki[wiki:summary oracle]
Gap -->|exists| Skip[Skip or refresh policy]
Wiki --> Synth[LLM synthesis w template]
Synth --> VW[vault:write]
VW --> Mem[Optional memory commit]
end

B.3 Architectural placement in eris

Not new tools for the core loop (initially): reuse wiki:summary, vault:read, vault:list, vault:write, and optionally memory: as today. The Gardener is orchestrator logic (new module or sub-task), analogous to how docs/ADDING_A_TOOL.md describes background events: respect TUI relay, no blocking, no second hidden queue for the same semantics — any new background task should follow established patterns (e.g. try_send, tracing).

State / when it runs: Define explicitly: e.g. only when AgentState::Idle and user idle timeout exceeded; or explicit user toggle in config. Avoid running heavy LLM + HTTP work during active Chat without consent.

**Idle / heartbeat vs future Sleep or Yield (do not fix piecemeal here):** Today’s `idle_timeout_secs` + `watch`-based heartbeat can send an interrupt while the user is away; the next `step()` may hit `interrupt_rx.changed()` before the LLM returns (`FcpError::Interrupted`). That is **not** “stuck in Reflect” — it is **idle machinery** colliding with the first turn after a long gap. Short-input conversational routing can still succeed. A proper fix belongs with **explicit separation of human-idle vs scheduler-yield** — see [SLEEP_MODE.md](./SLEEP_MODE.md) (fourth JSON status / `Sleep` or `Yield`) and the Gardener trigger design in this doc. Track integration there; avoid one-off heartbeat tweaks unless they are clearly subsumed by that model.

Gatekeeper: Gardener executions may run as internal tool invocations with appropriate state, or a dedicated internal path that still respects vault validation (validate_path_is_mutable, 00_Core immutability) — design decision when implementing; must not bypass security checks.

B.4 Zettel template (contract)

Each new 20_Semantic note should follow a stable, parseable shape so humans and future automation agree:

Suggested sections (adjust names in implementation):

Title — human-readable concept name.

Tags — comma-separated or YAML frontmatter (concurrency, rust, architecture, …).

Definition — short paraphrase + explicit (Source: Wikipedia) and link/URL from Phase 1 envelope.

Contextual gravity — 1–3 paragraphs: how this user / this vault uses or encountered the concept; cite episodic paths or agenda items by reference, not vague claims.

See also — optional wikilinks to other vault notes.

Forbidden as the sole content: unmodified full Wikipedia extract without synthesis section.

B.5 Slugs, dedup, idempotency

Stable slug: Derive filename from normalized concept key (e.g. lowercase snake_case from primary title). Handle collisions (disambiguation parenthetical in title → different slug).

Dedup: Before oracle call, if 20_Semantic/{slug}.md exists and freshness policy passes (e.g. younger than N days, or manual “pinned”), skip.

Refresh: Optional second phase feature: re-run oracle if note is stale and episodic still mentions concept; merge via LLM with diff discipline (show user or log).

B.6 Consent, surprise, inbox

First versions: Prefer writing to **Inbox/** or a 20_Semantic/\_staging/ tree, or emitting a \*\*TUI alarm / log entry for user review before promotion — reduces “why did my vault change overnight?”

Config knobs (future): gardener_enabled, gardener_max_notes_per_night, gardener_min_idle_secs, gardener_target_paths, optional allowlist of folders to scan.

B.7 Budget, rate limits, failure modes

Cap Wikipedia calls per Gardener run (429/403 handling: backoff, log tracing::warn, skip).

Cap LLM tokens per run; truncate episodic context with clear priority (today first).

Partial failure: one failed zettel must not abort the whole batch unless configured strict.

B.8 Licensing and attribution

Prefer short attributed definitions + URL from the Phase 1 envelope.

Long verbatim quotes from Wikipedia may trigger CC BY-SA obligations; synthesis-first design minimizes copy-paste surface. Product/legal review if distributing vault contents.

B.9 Risks (explicit)

Risk

Mitigation

Synthesis hallucinates project facts

Ground “Contextual gravity” in quoted excerpts from episodic files the Gardener actually read; user review / inbox

Vault spam

Dedup, caps, inbox, opt-in

Semantic brain noise

Only commit notes that pass template validation; optional human gate

Idle definition wrong

Config + clear logging when Gardener runs

B.10 Phase 2 implementation backlog (suggested order)

Vault conventions doc — 10_Episodic, 20_Semantic, slug rules (human + agent readable); optional template file in vault seed.

Gardener trigger — idle detection hook in orchestrator (exact integration TBD when coding).

Episodic reader — bounded scan, newest-first, max bytes.

Gap detector — filesystem existence + optional frontmatter concept_id.

Synthesis + vault:write pipeline — template enforcement in code (reject output missing required sections).

Optional memory commit — wire to existing memory tools after write.

Tests — tempfile vaults only; unit tests for slug/dedup; integration test with mock LLM or golden synthesis output.

Part C — shared “out of scope until specified”

Other-language Wikipedias; Wikidata; full article HTML fetch as part of wiki:summary.

Replacing Phase 1 envelope format in a breaking way without updating Phase 2 synthesis prompts.

Document control

Canonical plan path: /Users/jandahlke/.cursor/plans/wikipedia_summary_tool_ed6d8a81.plan.md

If wiki-summary*phased*\*.plan.md exists, treat it as deprecated; open this file instead.
