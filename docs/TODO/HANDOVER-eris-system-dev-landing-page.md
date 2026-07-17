# Handover: eris-system.dev landing page (M0 weekend launch)

**Goal:** A small, static, informative one-pager at **https://eris-system.dev** — the stable URL every post, README, and HN thread links to. Not a marketing funnel; an honest introduction to what Eris is, who builds it, and where the project stands.

**Companion docs:** [12_META_STRATEGY.md](../updated_architecture/12_META_STRATEGY.md) (positioning, launch sequence), [11_OSS_SHIPPING_ROADMAP.md](../updated_architecture/11_OSS_SHIPPING_ROADMAP.md) (M0/M1 gates), [README.md](../../README.md) (canonical copy).

**Owner:** Jan Paul Dahlke · **Repo:** [github.com/janpauldahlke/eris](https://github.com/janpauldahlke/eris) · **License:** Apache 2.0

---

## 1. Repo strategy: separate repo (recommended)

| Approach | Verdict |
|---|---|
| **New repo** `eris-system.dev` or `eris-site` | **Recommended.** Zero Rust toolchain, fast Cloudflare Pages deploy, no coupling to eris CI. The main repo links here; this repo links back to GitHub. |


**Suggested new repo layout:**

```text
eris-system.dev/
├── index.html              # single-page site (or Astro/11ty if you prefer components)
├── devlog/
│   └── index.html          # dev log listing (can start as section on index, split later)
├── assets/
│   ├── screenshots/        # operator-provided (TUI, web console, vault)
│   ├── demo.gif            # placeholder until vhs/asciinema recording lands (post-M0)
│   └── og-image.png        # 1200×630 social card
├── css/
│   └── theme.css           # design tokens (see §4)
├── _headers                # Cloudflare Pages security headers (optional)
├── wrangler.toml           # only if using Workers; Pages can deploy from Git directly
└── README.md               # deploy notes only
```

**Cloudflare Pages:** connect the new GitHub repo → build command empty (static) → output `/` → custom domain `eris-system.dev`.

---

## 2. What the page must do (M0 bar)

Per meta strategy §5, M0 needs only:

- [ ] One-line pitch (above the fold)
- [ ] GitHub link (primary CTA)
- [ ] Three value props (sovereign / grammar-disciplined / vault-native)
- [ ] Reliability & tools strip (recovery / gatekeeper / extensible tools / Rust) — §3.3
- [ ] “What Eris is not” (redirect ChatGPT-comparison crowd)
- [ ] Status badge: **v0.1.0-alpha · macOS-first · Apache 2.0**
- [ ] Dev log section (even one entry: “going public”)
- [ ] Footer: no tracking, privacy posture, security contact, copyright

**Explicitly NOT required for M0 launch:**

- Install one-liner (M1 — installers not ready)
- TUI GIF (record after truncation fix; use screenshot + “demo coming” note)
- Email list / Buttondown (add only if you will send updates)
- Blog subsystem (essays can be `/devlog` markdown pages added incrementally)

**M1 additions (later):** install block, benchmark table embed, GitHub Sponsors button, full GIF hero.

---

## 3. Information architecture (single scrolling page)

Top → bottom. Each `#section` gets an anchor for deep links.

### 3.1 Hero

**Headline (H1):** Eris

**Subhead (one line — use everywhere):**
> A local-first agent in a single Rust binary: your Markdown vault as memory, grammar-enforced tool calls on llama.cpp, nothing leaves your machine unless you say so.

**Secondary line (expansion):**
> A personal agent over your Obsidian-compatible vault — reads and writes notes, remembers across sessions through tiered memory, and calls tools through a JSON protocol structurally enforced by GBNF grammars.

**About the name (short section or footer — not hero subtitle):**
> **Eris** is named for the Greek goddess of strife and the golden apple of discord — not an acronym. A local agent that productively disturbs your vault: read, write, remember, remind.

**Primary CTA:** View on GitHub → `https://github.com/janpauldahlke/eris`

**Secondary CTA:** Architecture docs → `https://github.com/janpauldahlke/eris/tree/main/docs/updated_architecture`

**Status pill:** `Alpha · v0.1.0-alpha · macOS-first · Apache 2.0`

**Hero media:** Web console screenshot in **light mode** — reference asset at `docs/TODO/assets/web-console-light.png` in the eris repo (copy to new site as `assets/screenshots/web-console-light.png`). Shows chat + THOUGHT + TELEMETRY + status bar. Alt: “Eris web console — chat, thought panel, telemetry, and status metrics.”

---

### 3.2 Three pillars (value props)

Three cards, equal weight. Icons optional (simple line icons or monospace glyphs).

| Pillar | Title | Body |
|---|---|---|
| 1 | **Sovereign by architecture** | No telemetry. Your vault is plain Markdown you own. Every outbound integration — web fetch, mail, calendar, Discord — is opt-in and enumerable. The semantic index is derived data; rebuild it anytime from your notes. |
| 2 | **Grammar-disciplined tools** | Local 8–26B models are sloppy with JSON. Eris compiles GBNF grammars per session — and narrowed per-turn subset grammars — so output is constrained at the token level. ~63 tools pass through one **Gatekeeper** (JSON Schema + per-state allowlists). No function-calling API required. |
| 3 | **Vault-native memory** | Your working directory *is* the vault. Tiered recall: ephemeral staging (moka), committed notes on disk, semantic search (Qdrant). Prefetch and reindex-on-write keep the agent aligned with what you actually edited. |

---

### 3.3 Reliability & tools (supporting points — four short cards or a 2×2 grid)

These sit *under* the three pillars. They answer “why does this work on small local models?” and “can I extend it?” — do not promote them into the hero.

| Point | Title | Body (use this copy) |
|---|---|---|
| 1 | **Self-healing recovery** | Small models still drift: wrong tool args, tool I/O faults, incomplete turns. Eris treats those as **recoverable protocol events**, not crashes. A bounded **Recover** loop feeds the model a concrete correction (what failed, what shape is expected) and retries — capped by `max_recovery_attempts`. Grammar prevents most malformed JSON; recovery catches what remains. |
| 2 | **Gatekeeper-controlled tools** | Every tool call goes through one gate: **state allowlist** (Chat / Reflect / Idle / Recover each sees a different set) → **arg normalization** (common LLM key mistakes) → **JSON Schema validation** → execute. Operators can enable/disable tool families in config and from the web Tools console. The gatekeeper is a *protocol* boundary for the model — not a security sandbox against a malicious local user (say that once, honestly). |
| 3 | **A tool roster you can grow** | Out of the box: vault read/write/list, memory stage/commit/query, agenda & alarms, web fetch/search (allowlist + consent), weather, wiki, system health — plus optional vision, mail/calendar, Discord. New tools are a first-class extension path: implement the `Tool` trait, register with the gatekeeper, add a descriptor TOML, allowlist by state. See [ADDING_A_TOOL.md](https://github.com/janpauldahlke/eris/blob/main/docs/HOW_TO/ADDING_A_TOOL.md). |
| 4 | **Rust, on purpose** | One binary. Memory-safe by default: **no `unsafe`**, no panics in production paths (`?` + `FcpError`). Actor-model concurrency (`mpsc`, not shared `Mutex`) keeps the orchestrator responsive under tool rounds. Fast enough for a full-screen TUI and a localhost web console on the same machine as the model. |

**Example tool line** (optional one-liner under the roster card, monospace/telemetry style):

```text
[tool] vault:write · memory:query · agenda:remind_at · web:fetch · vision:see
```

---

### 3.4 One brain, three faces

Short section with three columns:

- **Terminal (ratatui)** — `eris chat` — full-screen TUI, primary dogfood surface
- **Web console** — `eris chat --web` — localhost Axum + SSE, same session
- **Discord (optional)** — sidecar sharing the live orchestrator

**Screenshot slots:** TUI + web console (operator provides). Discord optional.

---

### 3.5 How it works (minimal diagram)

A simple ASCII or SVG block — no heavy animation. Tools and memory are first-class — not buried under “orchestrator.”

```text
┌─────────────┐     ┌──────────────────┐     ┌─────────────┐
│  TUI / Web  │────▶│   Orchestrator   │────▶│ llama.cpp   │
│  / Discord  │◀────│  + Recover loop  │◀────│  (GBNF)     │
└─────────────┘     └────────┬─────────┘     └─────────────┘
                             │
              ┌──────────────┼──────────────┐
              ▼                             ▼
     ┌─────────────────┐          ┌─────────────────┐
     │ Gatekeeper      │          │ Memory          │
     │ → ~63 Tools     │          │ Vault (Markdown)│
     │ (schema+state)  │          │ Ephemeral(moka) │
     └─────────────────┘          │ Qdrant (embed)  │
                                  └─────────────────┘
```

One paragraph: same `SessionEvent` / `UserAction` layer across all surfaces. On each tool turn: GBNF constrains the JSON envelope → **Gatekeeper** checks state allowlist + schema → **tool** runs → failures enter Recover with a bounded retry. **Memory** is always in the loop: vault notes on disk, ephemeral staging for the session, Qdrant for semantic recall. Surfaces stay thin; the orchestrator owns the loop.

---

### 3.6 What Eris is — and is not

**Is:**
- A **Rust** reference architecture for sovereign local LLM pipelines: actor model, zero `unsafe`, zero panics in production paths, single binary
- The most reliable harness for the GGUF you can run locally — grammar + gatekeeper + recovery, not “smarter model”
- Smarter *about your data* than anything hosted — lives where your notes live
- Extensible: add tools without forking the orchestrator

**Is not:**
- Smarter than hosted frontier models (intelligence ceiling = your local model)
- A private ChatGPT replacement (wrong comparison; wrong expectations)
- A security sandbox (gatekeeper = protocol boundary for the LLM, not anti-malware)

**Hardware honesty table** (from README):

| Setup | Experience |
|---|---|
| Apple Silicon, 16 GB+ | Good: 7–12B chat + embed, Metal |
| Apple Silicon, 32 GB+ | Comfortable: 26B-class, vision mmproj |
| Linux + NVIDIA 8 GB+ VRAM | Good with GPU layer tuning |
| CPU-only | Runs; small quants only; slow |

---

### 3.7 What’s in the box (not “core vs extras”)

**Do not** use a “Core / Extras (best-effort)” table on the landing page — it demotes vision/voice right after the senses section.

**Section title:** What’s in the box  
**Subhead:** Same binary. Different layers of setup.

| Layer | What | When |
|---|---|---|
| **Always on** | Chat (TUI + web), vault read/write/list, Gatekeeper + GBNF (llama.cpp), tiered memory (ephemeral + vault + Qdrant), agenda / alarms | First `eris chat` after wizard + Qdrant |
| **Opt-in** | Web fetch (browser39, allowlist + consent), weather, wiki, Discord sidecar, Google mail/calendar, Moltbook | Config flag / credentials |
| **Hardware-gated** | Vision (`vision:see`), voice ingress (STT) | Multimodal GGUF + mmproj / ffmpeg; ~16 GB VRAM stack documented above |

**Footer line under table:**
> Opt-in and hardware-gated features ship in the same binary. They are not afterthoughts — they are just not required to chat with your vault.

**Backend note:** llama.cpp is the canonical production backend. Ollama is easier install with weaker JSON discipline.

*(README may still say “core vs extras” for contributors — landing page uses this three-layer frame.)*


---

### 3.8 Quickstart (honest, M0)

Do **not** promise one-click install yet. Show the real path:

```bash
# Qdrant (semantic memory)
docker run -d -p 6333:6333 -v eris-qdrant-data:/qdrant/storage qdrant/qdrant

# Build (requires Rust, llama-server, GGUF models — see docs)
cargo build --release
./target/release/eris chat          # first-run wizard
./target/release/eris chat --web    # same session, web UI
```

Link: [docs/REFERENCE.md](https://github.com/janpauldahlke/eris/blob/main/docs/REFERENCE.md) · [llama.cpp setup](https://github.com/janpauldahlke/eris/blob/main/docs/HOW_TO/LLAMA_CPP_SETUP.md)

**Known limitations callout (visible, not buried):**
- Alpha; single-user, single-process
- Installation is manual; installers planned (M1)
- Long-context JSON discipline: improving; benchmark table coming
- Developed primarily on macOS

---

### 3.9 Dev log (dedicated section)

**Purpose:** The engineering diary *is* the marketing (meta strategy §4). This section grows; launch with one entry.

**URL:** `#devlog` on index, or `/devlog/` when split.

**Entry template:**

```markdown
### 2026-07-17 — Going public (M0)

We're open-sourcing Eris this weekend under Apache 2.0. The codebase is real —
~64k lines of Rust, dogfooded daily — not a demo. M0 is "public source":
you can read, build, and judge. M1 (installable in ~15 minutes) comes next.

What's in: GBNF-enforced tool protocol, tiered vault memory, TUI + web + optional
Discord on one orchestrator.

What's out for now: installers, prebuilt binaries, published benchmark tables.
The truncation-under-long-context issue is documented honestly; fix lands before
the visibility push to r/LocalLLaMA.

— Jan
```

Future entries (cadence 2–3 weeks): GBNF essay, actor-model post, memory tier design, audit meta-post. Link each to GitHub discussions or `/devlog/YYYY-MM-DD-slug.html`.

---

### 3.10 Links & community

- **GitHub:** https://github.com/janpauldahlke/eris
- **Issues / discussions:** GitHub (no Discord requirement for M0)
- **Security:** [SECURITY.md](https://github.com/janpauldahlke/eris/blob/main/SECURITY.md) · report to janpauldahlke@gmail.com
- **Contributing:** [CONTRIBUTING.md](https://github.com/janpauldahlke/eris/blob/main/CONTRIBUTING.md) · DCO sign-off
- **Sponsors:** GitHub Sponsors (add URL when live)
- **Canonical tag for search:** `#eris-agent` (name collision with Go eris, Discord libs — use compound)

---

### 3.11 Footer

```text
© 2026 Jan Dahlke · Apache 2.0

This site loads no analytics, no cookies, no third-party trackers.
That is intentional and on-brand.

Built with static HTML on Cloudflare Pages.
Source: [link to eris-system.dev repo when public]
```

---

## 4. Design system (match Eris web console — light mode)

**Default theme:** **light** (operator choice — see reference screenshot `docs/TODO/assets/web-console-light.png`). The landing page should feel like you opened the web console in a browser tab, not like a separate marketing brand. Optional dark toggle is nice-to-have, not M0.

Source of truth: `src/ui/web/templates/chat.html` → `[data-theme="light"]` block.

### 4.1 Color tokens (light — use as `:root` on the site)

```css
:root {
  /* Surfaces */
  --bg: #e8ecf4;              /* page background — soft blue-grey */
  --panel: #ffffff;           /* cards, chat area, screenshot frame inner */
  --footer-bg: #dce3f0;       /* header bar, footer strip */
  --rail-bg: #d0d9ea;         /* optional left nav accent (site header) */
  --thought-bg: #f4f6fb;      /* dev-log entry background, blockquotes */
  --chip-bg: #e8ecf4;         /* status pills, inline tags */
  --telemetry-bg: #eef2f9;    /* secondary panels, code-adjacent blocks */
  --input-bg: #ffffff;

  /* Text */
  --text: #1a2238;            /* body copy */
  --text-strong: #0b1220;     /* headings, emphasis */
  --muted: #4a5568;           /* secondary text, captions */
  --assistant-text: #1a2238;
  --telemetry-text: #3d4a62;

  /* Borders */
  --border: #a8b4cc;          /* primary dividers */
  --telemetry-line-border: #c5cede;
  --status-chip-border: #b8c4d8;

  /* Brand / interactive */
  --accent: #0a6b55;          /* links, primary CTA, “chat” state — deep teal */
  --user: #1456a8;            /* user-facing highlights, [WEB]-style tags */
  --btn-submit-bg: #e6f5f0;
  --btn-submit-hover: #d0ebe3;

  /* Semantic / telemetry accents (from console rail) */
  --tool-activity: #9a6b00;   /* tool lines, amber */
  --tel-tool-color: #7a5a10;
  --tel-tool-border: #c9a030;
  --tel-sys-color: #1d5fa8;   /* system telemetry — blue */
  --tel-sys-border: #4a8fd4;
  --tel-fcp-color: #0a6b55;   /* fcp / core logs — matches accent */
  --system-msg: #8a6b20;

  /* Agent state pills (reuse for status badges on site) */
  --state-idle: #6a738c;
  --state-chat: #0a6b55;
  --state-reflect: #6b4fa8;
  --state-recover: #b86e1a;
  --status-pill-idle-bg: #f0f3fa;
  --status-pill-idle-border: #b8c4d8;
  --status-pill-chat-bg: #e6f5f0;
  --status-pill-chat-border: #7ab8a8;

  /* Code */
  --markdown-code-bg: #e8ecf4;
  --markdown-code-border: #b8c4d8;
  --markdown-strong: #0b1220;
}
```

**Dark theme (optional later):** same file, `[data-theme="dark"]` — accent flips to mint `#5ce5be` on near-black `#080a12`. Only add if you want a toggle; do not ship dark as default.

### 4.2 UX patterns (borrow from the console, adapt for marketing)

The web UI is **informative and instrumented** — the landing page should echo that without cloning the full layout.

| Console element | Landing-page adaptation |
|---|---|
| Top bar (`ERIS · Gem @ gemma12B`) | Sticky site header: wordmark **ERIS** + muted subtitle “local-first vault agent” + nav anchors (About · Dev log · GitHub) |
| `[gem]` / `[WEB]` message tags | Use for dev-log entries: `[devlog]`, `[release]`, or date tags in `--chip-bg` boxes |
| THOUGHT panel (grey inset boxes) | Blockquotes, dev-log intros, or “design note” callouts — `--thought-bg`, left border `--border` |
| TELEMETRY rail (amber `[tool]` lines) | Feature lists or “under the hood” bullets styled like log lines: monospace, `--tel-tool-color`, small caps label |
| Status bar chips (`IDLE`, `ROUNDS 0/40`, token counts) | Hero **status pill**: `Alpha · v0.1.0-alpha · macOS-first` using `--status-pill-idle-bg` / `--status-pill-idle-border` |
| Green Send button | Primary CTA (“View on GitHub”) — `--btn-submit-bg`, hover `--btn-submit-hover`, text `--accent` |
| White chat panel on grey field | Section cards: `--panel` on `--bg`, 1px `--border`, no drop shadows (console uses flat borders, not elevation) |
| Embedded images in chat | Screenshot sits in a “panel” frame — max-width 100%, border `--border`, optional caption in `--muted` |

**What to avoid:** purple gradients, glassmorphism, Inter/Roboto “SaaS” fonts, giant hero illustrations, animation-heavy scroll effects. The screenshot *is* the hero art.

### 4.3 Typography

- **Family:** `ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace` — same as web UI (this is intentional; it signals “developer tool,” not consumer app)
- **Headings:** weight 600, `--text-strong`; H1 can be slightly larger (1.75–2rem) but stay monospace
- **Body:** 15–16px (web UI uses 14px — bump slightly for long-form reading on the landing page)
- **Line height:** 1.6 prose, 1.4 for dense tables/code
- **Labels / tags:** 12–13px, letter-spacing optional +0.02em for `[tag]` prefixes

### 4.4 Layout

- **Page:** `--bg` full bleed; content column max-width **840px** centered; hero screenshot can break out to **960–1100px**
- **Section rhythm:** 4–5rem between major sections; hairline `--border` optional between pillars
- **Cards / tables:** `--panel` fill, 1px `--border`, border-radius **4px** (match console — subtle, not pill-shaped cards)
- **Nav:** compact header, `--footer-bg` or `--panel` background, bottom border `--border`
- **Mobile:** stack columns; status metrics become wrapped chips; screenshot scales down; no horizontal scroll on body text

### 4.5 Components (CSS snippets for agent)

**Status pill (hero):**
```css
.status-pill {
  display: inline-block;
  padding: 0.25rem 0.75rem;
  font-size: 0.8125rem;
  background: var(--status-pill-idle-bg);
  border: 1px solid var(--status-pill-idle-border);
  border-radius: 4px;
  color: var(--muted);
}
```

**Primary button:**
```css
.btn-primary {
  background: var(--btn-submit-bg);
  border: 1px solid var(--status-pill-chat-border);
  color: var(--accent);
  padding: 0.5rem 1rem;
  border-radius: 4px;
  font-family: inherit;
  text-decoration: none;
}
.btn-primary:hover { background: var(--btn-submit-hover); }
```

**Telemetry-style feature line:**
```css
.feature-line {
  font-size: 0.875rem;
  color: var(--tel-tool-color);
  border-left: 3px solid var(--tel-tool-border);
  padding-left: 0.75rem;
  margin: 0.5rem 0;
}
```

**Screenshot frame:**
```css
.screenshot-frame {
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 0.5rem;
}
.screenshot-frame img {
  display: block;
  width: 100%;
  height: auto;
  border-radius: 2px;
}
```

### 4.6 Tone & accessibility

- **Tone:** Technical diary / operator manual — the console already looks like one
- **Contrast:** `--accent` `#0a6b55` on white passes WCAG AA for links; verify `--muted` on `--bg` for captions
- **Focus:** visible focus ring using `--user` or `--accent` outline
- **Semantics:** `main`, `nav`, `article` per dev-log entry; `figure`/`figcaption` for screenshot

---

## 5. SEO & metadata

```html
<title>Eris — local-first agent for your Markdown vault</title>
<meta name="description" content="Eris is a local-first agent in Rust: Obsidian-compatible vault as memory, GBNF-enforced tool calls on llama.cpp. Nothing leaves your machine unless you say so." />
<meta name="keywords" content="eris-agent, local LLM, llama.cpp, GBNF, Obsidian, Rust, sovereign AI" />
<link rel="canonical" href="https://eris-system.dev/" />

<!-- Open Graph -->
<meta property="og:title" content="Eris — local-first vault agent" />
<meta property="og:description" content="Grammar-enforced tools. Markdown vault as memory. Single Rust binary." />
<meta property="og:url" content="https://eris-system.dev/" />
<meta property="og:image" content="https://eris-system.dev/assets/og-image.png" />
<meta property="og:type" content="website" />

<!-- JSON-LD (SoftwareSourceCode) -->
<script type="application/ld+json">
{
  "@context": "https://schema.org",
  "@type": "SoftwareSourceCode",
  "name": "Eris",
  "alternateName": "eris-agent",
  "description": "Local-first agent: Markdown vault as memory, GBNF-enforced tool calls on llama.cpp.",
  "codeRepository": "https://github.com/janpauldahlke/eris",
  "programmingLanguage": "Rust",
  "license": "https://www.apache.org/licenses/LICENSE-2.0",
  "author": { "@type": "Person", "name": "Jan Paul Dahlke" }
}
</script>
```

---

## 6. Assets checklist (operator)

| Asset | Spec | Status |
|---|---|---|
| Web console screenshot (light) | Chat + THOUGHT + TELEMETRY + status bar | **Done** — `docs/TODO/assets/web-console-light.png` → copy to site repo |
| TUI screenshot | ratatui session, ideally showing tool call or memory | **TODO** |
| Vault / memory diagram | Optional; screenshot of Obsidian tree + `.fcp/` | Optional |
| `demo.gif` | 30–60s TUI recording (vhs/asciinema) | Post-M0; placeholder box OK |
| `og-image.png` | 1200×630, **light** `--bg` + `--accent` tagline | Generate from hero palette |
| Favicon | Simple “E” or terminal glyph, 32×32 + 180×180 apple-touch | TODO |

Store in `assets/screenshots/` with descriptive names. Prefer WebP with PNG fallback.

---

## 7. Cloudflare Pages deploy checklist

1. Create GitHub repo `eris-system.dev` (or org account if you prefer)
2. Cloudflare Dashboard → Pages → Connect Git → select repo
3. Build settings: **None** (static) · Output directory: `/`
4. Custom domain: `eris-system.dev` (+ `www` redirect to apex)
5. SSL: Full (strict) — automatic on Cloudflare
6. Optional `_headers`:
   ```text
   /*
     X-Frame-Options: DENY
     X-Content-Type-Options: nosniff
     Referrer-Policy: strict-origin-when-cross-origin
     Permissions-Policy: interest-cohort=()
   ```
7. After deploy: add `https://eris-system.dev` to eris `README.md`, `Cargo.toml` homepage field, GitHub repo website URL

---

## 8. Cross-links back to main repo (after launch)

In eris `README.md` (human applies — not agent scope unless asked):

- Replace `<!-- TODO: 30–60s TUI demo GIF here before public launch -->` with link to eris-system.dev
- Add “Website: https://eris-system.dev” under badges
- Meta strategy action item: register `eris-agent` GitHub topic

---

## 9. Agent prompt (copy-paste into clean repo)

Use this verbatim in a fresh Cursor session opened in the new `eris-system.dev` repository.

---

```
You are building the static landing page for Eris (eris-system.dev).

## Context
Eris is an open-source (Apache 2.0) local-first AI agent in a single Rust binary.
GitHub: https://github.com/janpauldahlke/eris
The site is NOT the app — it is a small, honest introduction and stable URL for launch weekend (M0).

Read the full spec in the handover doc (paste or attach HANDOVER-eris-system-dev-landing-page.md from the eris repo docs/TODO/).

## Hard requirements
- Static site only — no backend, no analytics, no cookies, no third-party trackers
- Deploy target: Cloudflare Pages (plain HTML/CSS/JS or minimal Astro; no React SPA)
- Single scrolling page with anchored sections + dedicated #devlog section
- Match Eris web console **light theme** (design tokens + UX patterns in spec §4)
- Monospace typography throughout
- Mobile-responsive (web UI assumes 1024px; landing page must work on phone)
- All copy from spec §3 — do not invent marketing fluff or “ChatGPT replacement” framing
- Include “What Eris is not” and honest alpha limitations
- Primary CTA: GitHub repo link
- Footer states no tracking explicitly

## Content sections (in order)
1. Hero — pitch, status pill, GitHub CTA, screenshot
2. Three pillars — sovereign / grammar-disciplined / vault-native
3. Reliability & tools — self-healing recovery, Gatekeeper, tool roster + write-your-own, Rust (safe + fast)
4. One brain three faces — TUI, web, Discord
5. Architecture diagram (simple SVG or preformatted)
6. What Eris is / is not + hardware table
7. Core vs extras table
8. Quickstart (honest manual install, not fake one-liner)
9. Dev log — seed with 2026-07-17 “Going public” entry
10. Links — docs, security, contributing
11. Footer — copyright, Apache 2.0, no-tracking note

Use §3.3 copy verbatim for the four reliability cards. Do not invent “self-healing AI” marketing — recovery is a bounded protocol loop for small-model failures.

## Design (light mode — see spec §4)
- Page bg #e8ecf4, panels #ffffff, accent #0a6b55, text #1a2238, borders #a8b4cc
- Font: ui-monospace, Menlo, Monaco, Consolas — everywhere
- UX: status pills, telemetry-style feature lines, thought-box callouts — not generic SaaS
- Hero screenshot: use provided web-console-light.png in a screenshot-frame
- Flat borders, no drop shadows, 4px radius — match the console
- Use CSS variables from spec §4.1; component snippets in §4.5

## Assets
- Copy web-console-light.png to assets/screenshots/ (provided by operator)
- TUI screenshot: placeholder OK for M0
- og-image.png: 1200×630, light palette (#e8ecf4 bg, #0a6b55 accent text)
- Favicon: minimal “E” glyph in teal on light grey

## Deliverables
- index.html (or Astro equivalent)
- css/theme.css
- assets/ structure per spec
- README.md with Cloudflare Pages deploy steps
- Valid semantic HTML, meta tags, JSON-LD from spec §5

## Do NOT
- Add npm dependencies unless strictly necessary (prefer zero-build static)
- Add cookie banners (there are no cookies)
- Claim “structurally impossible JSON” — use “structurally enforced”
- Promise installers or one-click setup (M1)
- Use stock AI imagery or purple gradient heroes

When done, run a local server and verify all anchor links, mobile layout, and contrast.
```

---

## 10. Submodule alternative (if you choose it)

Only if you want the site inside the eris monorepo:

```bash
# From eris repo root, after creating eris-system.dev repo
git submodule add https://github.com/janpauldahlke/eris-system.dev.git site
```

Cloudflare Pages can still deploy from the submodule path (`site/`) via monorepo config, but separate repo is simpler.

---

## 11. Launch weekend sequence

| When | Action |
|---|---|
| Fri | Create repo, agent builds skeleton, you capture screenshots from running web UI |
| Sat | Drop assets, review copy, deploy to Cloudflare, smoke-test on phone |
| Sun | M0 tag `v0.1.0-alpha` on eris, add website URL to GitHub, soft mention only (no HN yet — meta strategy gates HN on M1 + truncation fix) |

---

*Handover written 2026-07-17. Update dev log entry dates and version strings as milestones land.*
