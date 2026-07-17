# Meta strategy — does eris hold, and how to make it visible

Companion to [10_DEEP_REVIEW_2026-07.md](./10_DEEP_REVIEW_2026-07.md) (what's inside) and [11_OSS_SHIPPING_ROADMAP.md](./11_OSS_SHIPPING_ROADMAP.md) (how to release it). This file: whether the product claim survives contact with reality, and how a developer-not-salesman gets it in front of people without becoming a salesman.

Method as requested: second-order observation — not "is eris good" but "what does eris look like from outside, to someone who owes you nothing."

---

## 1. Does eris hold what it proposes?

Judged claim-by-claim against the README front matter and the audited code.

| Claim | Verdict | Evidence / caveat |
|---|---|---|
| "Local, vault-centric assistant" | **Holds.** | Genuinely local-first: peripherals spawned and reaped locally, Qdrant local, no telemetry egress. Vault is first-class (ingest, reindex-on-write, prefetch), not a bolt-on. |
| "Same orchestrator and tools across TUI / web / Discord" | **Holds.** | One `SessionEvent`/`UserAction` presentation layer, one gatekeeper, verified in audit. This is architecturally unusual and real. |
| "GBNF grammar makes malformed JSON structurally impossible" | **Overclaims — the weak point.** | Grammar guarantees *shape*, not *completion*. On long contexts, `n_predict: -1` + no completion-window reservation ⇒ truncated envelopes (deep review §3). The sentence in README line 47 is falsifiable by any r/LocalLLaMA user with a long session. **Fix the code before launch, and soften the claim to "structurally enforced" until then.** |
| "Tools run only through the JSON-schema gatekeeper" | **Holds, with asterisk.** | True for the orchestrator path. Chat-state allowlist is nearly open and Recover elevates to Chat — fine as design, but don't market the gatekeeper as a security boundary; it's a *protocol* boundary. |
| "Two LLM backends" | **Holds but shouldn't.** | Both work; they have divergent reliability semantics. Roadmap already decides: llama.cpp canonical, Ollama experimental. Two backends is a maintenance claim, not a feature claim. |
| Memory (semantic + ephemeral + prefetch + reindex) | **Holds.** | The tier design is coherent and tested. Unproven at scale (nobody else's vault has hit it yet) — say "designed for," not "proven at." |
| Vision / voice on llama.cpp | **Holds, narrow.** | Model- and build-version-dependent (mmproj, b9493+). This is honest in the README already — keep it that honest. |
| The implicit claim: "digital sovereignty" | **Holds architecturally, unproven socially.** | All outbound surfaces are opt-in and enumerable. But sovereignty is a *trust* claim, and trust needs the SECURITY.md statement, opt-out defaults, and time. You can't assert it; you can only make it auditable. |

**Net verdict:** eris is a real system, not a demo — the architecture claims hold. The one claim that doesn't fully hold yet (JSON discipline under long context) is unfortunately the *headline* claim and the one your most likely early adopters will stress-test first. That single fact should sequence your entire launch: **the truncation fix ships before the visibility push.**

### The honest ceiling nobody puts in a README

Eris's intelligence ceiling is the local model's ceiling. A disciplined pipeline around an 8–26B GGUF still reasons like an 8–26B GGUF. Eris cannot win "smartest assistant"; it can win **"most reliable and most private harness for the model you can run."** Every piece of framing below follows from accepting that.

---

## 2. Positioning: what eris actually is

Three candidate framings, in descending order of honesty-per-watt:

1. **"A reference architecture for sovereign local LLM pipelines"** *(primary, dev-facing).* The codebase itself is the product: GBNF-enforced tool protocol, actor-model concurrency without mutexes, tiered memory, a gatekeeper, zero-panic Rust. Nobody else ships this combination as readable open source. This framing matches who you are (developer, not salesman) and what you can sustain (technical writing, not growth hacking).
2. **"Your Markdown vault gets hands"** *(secondary, user-facing).* Obsidian-adjacent people already believe in local files and plain text; an agent that reads, writes, remembers, and reminds *inside the vault* is a legible promise. Smaller audience than "AI assistant," but one you can actually satisfy.
3. **"Private ChatGPT replacement"** — *reject this framing.* It invites comparison you lose on model quality, and it attracts users whose expectations you can't meet. When commenters apply it to you (they will), redirect rather than argue.

**One-line pitch (use everywhere, iterate wording):**
> Eris is a local-first agent in a single Rust binary: your Markdown vault as memory, grammar-enforced tool calls on llama.cpp, nothing leaves your machine unless you say so.

**Naming risk (do this check now):** "eris" collides hard — the Go error library `eris`, the Eris Discord JS library, the dwarf planet, Ilya's Discordianism jokes. You don't need to rename; you need a canonical compound for search: **`eris-agent`** as the GitHub topic, domain, and hashtag. Verify `github.com/janpauldahlke/eris` discoverability against the collisions before printing the name on a landing page.

---

## 3. Audiences, in order of reachability

| Audience | Where | What they want from you | Conversion |
|---|---|---|---|
| **Local-LLM tinkerers** | r/LocalLLaMA, HN, llama.cpp discussions | GBNF war stories, benchmarks, GGUF configs that work | Users + issue reporters |
| **Rust developers** | r/rust, This Week in Rust, HN | Architecture writeups (actor model, no-unsafe, error taxonomy) | Contributors |
| **Obsidian / PKM people** | r/ObsidianMD, PKM forums, Discords | "Vault gets hands" demo GIF, safety guarantees about their notes | Users (less technical — need M1 installer first) |
| **Digital-sovereignty / EU tech discourse** | LinkedIn (DE/EN), Mastodon/Fediverse, netzpolitik-adjacent circles | The *argument*: local pipelines as practical sovereignty, not regulation talk | Reputation, talks, unusual allies |
| **Self-hosters** | r/selfhosted, Hacker News | docker-compose that works on their Linux box | Users (needs M2 full-stack compose) |

Sequence them: tinkerers and Rust devs first (they tolerate rough edges and file good bugs), PKM and self-hosters after M1/M2 when the install story is real. The sovereignty audience runs in parallel because it's essay-driven, not install-driven — and as a German developer you have native credibility in exactly the LinkedIn/Fediverse space where "digitale Souveränität" is currently a live debate. That's your only unfair distribution advantage; use it.

---

## 4. The content flywheel (the marketing you can actually sustain)

You said it yourself: developer, not salesman. Good news — in this niche, **the engineering diary is the marketing.** Every hard problem you already solved is a post that markets the repo without selling anything. You have at least six sitting in the audit trail:

1. **"Teaching a small model JSON discipline: GBNF grammars in production"** — the envelope grammar, per-turn subset grammars, what grammars can and cannot guarantee. *This is the launch essay.* It's differentiated, checkable, and ends naturally with "the code is open."
2. **"The bug where my grammar was perfect and my JSON still broke"** — the `n_predict: -1` truncation post-mortem (deep review §3). Post-mortems outperform feature posts; write it *after* the fix ships so it ends in a win.
3. **"A TUI agent with zero mutexes: actor-model Rust for LLM pipelines"** — the mpsc/watch architecture, why `Arc<Mutex<Orchestrator>>` is banned.
4. **"63 tools and one gatekeeper: schema-validated tool calls without function-calling APIs."**
5. **"Your notes as memory: tiered recall over an Obsidian vault with Qdrant"** — prefetch, reindex-on-write, promotion/decay.
6. **"What I learned auditing my own 64k-line codebase"** — meta-post, honest about the god component; vulnerability posts build more trust than victory posts.

Cadence: one essay per 2–3 weeks beats six in launch week. Each post: publish on your own domain (see §5), submit to the matching subreddit + HN yourself, cross-post the sovereignty-angled ones to LinkedIn in German and English.

---

## 5. Channels and artifacts

### Landing page (yes — build it, keep it tiny)

Cloudflare Pages, static, one page. Its job is not conversion funnels; it's being the **stable URL** every post links to, that you control, that outlives any Reddit thread.

Content, top to bottom: the one-line pitch → 30–60s TUI GIF (asciinema/vhs — this artifact matters more than everything else on the page) → three value props (sovereign by architecture / grammar-disciplined / vault-native) → install block (one command, post-M1) → GitHub + docs links → sponsor link → optional email list (Buttondown-class, only if you'll actually send updates). No cookie banner needed if you add no tracking — which is itself on-brand; say so in the footer.

Blog lives on the same domain (`/blog` or `blog.` subdomain). The essays are the traffic; the page is the funnel's floor.

### Launch sequence (interleaves with roadmap milestones)

| Step | Gate | Action |
|---|---|---|
| 0 | M0 done | Repo public, no announcement. Let it breathe a week — fix anything embarrassing that friends/first finders hit. |
| 1 | Truncation fix merged | Soft launch: one post in r/LocalLLaMA framed as *"I built a local agent that forces valid JSON out of small models via GBNF — architecture writeup"* (link essay #1, mention repo). This community forgives alpha quality and gives the best technical feedback. |
| 2 | Same week | r/rust post angled at the architecture (essay #3 or the repo directly). LinkedIn post (DE+EN) with the sovereignty framing. |
| 3 | M1 done (installer works) | **Show HN.** Title pattern: "Show HN: Eris – local-first agent in Rust; Obsidian vault as memory, GBNF-enforced tools." Be present in the thread all day; answer the model-quality skeptics with the §2 redirect, not defensiveness. HN before the installer exists = wasted shot; you get one. |
| 4 | Ongoing | Essay cadence (§4); r/ObsidianMD + r/selfhosted when their install paths (M1/M2) are real. Submit the repo to awesome-lists (awesome-rust, awesome-llm, awesome-selfhosted). |
| 5 | Opportunistic | Rust/local-AI podcasts and meetups take cold emails from people with shipped, unusual codebases. A Chaos-/GPN-/rust-meetup talk on "grammar-enforced agents" is well within reach and compounds. |

### Funding: GitHub Sponsors first, Patreon second

Honest read: **Patreon underperforms for developer tools** — it lives off parasocial creator relationships you don't have and don't want to maintain. **GitHub Sponsors** is native to your audience, shows up on the repo itself, has zero content obligation, and (currently) no platform fee. Set it up at M0 with two tiers ("coffee" / "keep the lights on") and a one-paragraph honest pitch: solo developer, local-first tooling, money buys maintenance time. Add Patreon later only if a non-GitHub audience (PKM people, sovereignty crowd) actually asks for it. **Never** gate features behind sponsorship — for a sovereignty-framed project, paywalled features are self-refuting. If revenue ever matters seriously, the honest models are paid support, paid setup, or a hosted convenience layer — decisions for after there are users.

---

## 6. Risks — the second-order view of *you*

- **R1: Launch-then-silence.** The most common solo-OSS failure is not a bad launch; it's the repo going quiet 3 weeks after a good one. Issues from strangers feel like attacks the first month. Pre-commit to a sustainable pace (e.g. triage twice a week, one essay per 2–3 weeks) and put it in CONTRIBUTING.md so expectations are set for both sides.
- **R2: Perfection-delay.** You waited this long to share partly because the codebase "grew blind." The deep review exists precisely so you *know* the flaws — that's license to ship with a known-issues list, not a reason to fix everything first. Only the truncation bug is launch-gating; the god component is not.
- **R3: The wrong first cohort.** If the "private ChatGPT" crowd arrives first, you'll get model-quality complaints you can't fix. Mitigation: hardware-requirements table and "what eris is not" section on the landing page. Explicitly: *not smarter than hosted frontier models; smarter about your data than anything hosted.*
- **R4: Benchmark vacuum.** r/LocalLLaMA will ask "how does GBNF tool-calling compare to native function calling / other harnesses?" You have a `benchmark/` subsystem — publishing even a modest, honest table (envelope validity rate with/without grammar, across 2–3 models) preempts this and is itself essay material.
- **R5: Solo bus factor as trust argument.** Sovereignty users are exactly the users who ask "what if you disappear?" Apache 2.0 + readable architecture docs *is* the answer — say it explicitly: the project is built to be forkable.

---

## 7. Success metrics (define them now, so vanity doesn't)

- **Real:** strangers who install and file a reproducible issue (that's a user); a PR from someone you don't know; a second maintainer within 12 months; essay traffic that returns to the repo.
- **Vanity:** GitHub stars (HN can 10× them in a day with zero users behind them), LinkedIn impressions, Discord member counts.
- Quarterly question, per Luhmann: *"What is the difference between how eris describes itself and what its users are observed doing with it?"* — the answer is the next quarter's roadmap.

---

## 8. Where the potential is (ranked closing judgment)

Written after the July 2026 review session, as the takeaway ranking across docs 10–13.

1. **Turn the reliability claim into measured evidence.** Eris's differentiator is "disciplined pipelines for small local models" — every competitor *asserts* reliability; nobody *measures* it. The ~4.5k-line `benchmark/` harness already exists and has never been pointed at the one question that matters publicly: **JSON envelope validity rate — grammar vs. no grammar, across context lengths, across 2–3 models.** One table does four jobs: validates the headline claim before strangers do, verifies whether `n_predict_max` moved the needle, becomes the centerpiece of launch essay #1, and preempts r/LocalLLaMA skepticism. ~2 days reusing existing code. Nothing converts effort into credibility at a better rate.
2. **Subtraction as the primary engineering strategy.** The tier-ladder deletion ([13](./13_MEMVID_AND_MEMORY_SIMPLIFICATION.md) §2), the Ollama demotion ([11](./11_OSS_SHIPPING_ROADMAP.md) §1), the core-vs-extras freeze — the aggregate matters more than the parts: eris at ~40k lines is a *better and sustainable* product for a solo maintainer. Every deleted subsystem is context returned to the maintainer's head — the actual scarce resource. For the next six months, "what can I remove" outranks "what can I add."
3. **The read-time memory inversion** ([13](./13_MEMVID_AND_MEMORY_SIMPLIFICATION.md) §2): stop deciding at write time what will matter (the promote ladder — unknowable), decide at read time what matters now (similarity × recency — what embeddings are for). More than a refactor: a better theory of agent memory, publishable, and the kind of design insight that gets a project noticed. Simmer, don't rush.

**Explicitly ranked low despite the audit:** the orchestrator god-component ([10](./10_DEEP_REVIEW_2026-07.md) §2). Most visible flaw, least urgent — it hurts contributors that don't exist yet, not the product. Fix lazily, when the first real external PR to `orchestrator/core/` arrives.

Compressed: **the code is done enough — the deficit is proof and reach, not features.** Measure the claim, publish the measurement, delete what you can, let the memory idea simmer. In that order.

---

## 9. Condensed action list

```text
Gate: M0 (this weekend)
[ ] GitHub Sponsors profile live
[ ] Register domain; Cloudflare Pages skeleton (pitch + GitHub link is enough)
[ ] "eris-agent" topic/tag decision after name-collision check
[ ] Soften README line 47 ("structurally impossible" → "structurally enforced")
    until the truncation fix lands

Gate: truncation fix merged
[ ] Record TUI GIF (vhs/asciinema)
[ ] Essay #1 (GBNF discipline) on own domain
[ ] r/LocalLLaMA soft launch + r/rust + LinkedIn (DE/EN)

Gate: M1 (installer)
[ ] Landing page complete (GIF, install one-liner, what-eris-is-not)
[ ] Show HN, full-day presence
[ ] Benchmark table published (grammar vs no-grammar envelope validity)

Ongoing
[ ] Essay every 2–3 weeks from §4 list
[ ] Triage rhythm in CONTRIBUTING.md; known-issues list honest and current
```
