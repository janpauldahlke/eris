# Memvid as a long-term memory backend + simplifying the promotion tiers

Companion to [10_DEEP_REVIEW_2026-07.md](./10_DEEP_REVIEW_2026-07.md) (§6 memory) and [11_OSS_SHIPPING_ROADMAP.md](./11_OSS_SHIPPING_ROADMAP.md). Two questions, answered together because they share one design decision:

1. Is memvid (v2, `memvid-core` crate) worth adding as a long-term memory choice?
2. Can the ephemeral promotion-tier machinery be simplified away from explicit write-time gating?

---

## 1. Memvid: honest assessment

### What it is now (and what it was)

Memvid v1 was the viral "store text as QR codes in video frames" project — a gimmick, and the maintainers have deprecated it themselves. **v2 is a different and much more respectable thing:** a single-file agent-memory store (`.mv2`) in Rust — embedded WAL, compressed append-only "Smart Frames", Tantivy BM25 full-text index, HNSW vector index, chronological time index, one file, no sidecar processes. Think "SQLite for agent memory," not "video codec magic." Apache 2.0, `memvid-core` on crates.io.

### Discount the marketing

The README claims "+35% SOTA on LoCoMo," "1,372× throughput," "0.025ms P50." Treat all of it as vendor benchmarks until reproduced. The honest evaluation criterion for eris is *not* recall quality (that's dominated by your embedding model and chunking, which stay the same) — it is **operational shape**.

### What memvid would actually buy eris

| | Qdrant (today) | memvid-core |
|---|---|---|
| Deployment | Separate daemon (Docker or native), gRPC port, spawn/reap in `peripherals.rs` | **In-process crate, one file on disk** |
| Onboarding cost | The single most annoying dependency in the M1 installer story | Zero — `cargo` dependency |
| Backup/portability | Volume snapshots, collection migrations | Copy one file |
| Full-text search | None (eris scans vault files lexically in `vault/search.rs`) | Tantivy BM25 built in |
| Recency queries | Payload index bolted on (`ensure_recency_payload_index`) | First-class time index |
| Maturity | Battle-tested, huge deployment base | Young v2, API stability unknown, single-file DB engines are hard to get durably right |
| Dependency weight | gRPC client crate | Tantivy + HNSW + (optionally) ONNX runtime — heavy |
| Concurrency | Server handles concurrent access | Single-process file ownership (fine for eris — single binary is the design) |

**The killer argument is the first row.** Eris's shipping story (roadmap M1) suffers most from "install Docker to run Qdrant to run a *local-first sovereignty* tool." An in-process store dissolves that contradiction. The vector math is commodity; the deployment shape is the product.

### The two make-or-break technical questions (spike before deciding)

1. **Can eris supply its own vectors?** Eris's embeddings come from the managed `llama-server --embedding` (nomic-embed, 768-dim). Memvid's `vec` feature ships its own ONNX embedders (BGE et al.) behind an `EmbeddingProvider` trait. If the API accepts externally computed vectors (or a custom `EmbeddingProvider` impl backed by llama-server), we're fine. If it insists on running its own ONNX model in-process, that's a second embedding model in RAM and a violation of "the engine owns inference" — disqualifying as-is.
2. **Durability under crash + scale of your vault.** Append-only WAL claims need a smoke test: kill -9 during ingest, reopen, verify. Then ingest the real vault corpus and compare search latency + file size against the Qdrant collection.

Also verify: 768-dim support in its HNSW config (README examples are 384-dim BGE-small), and license hygiene of the ONNX feature if unused (keep it feature-flagged off).

### Verdict

**Worth it — as an experimental second backend behind a trait, not as a replacement.** This is precisely the redesign trigger that [09_CRITICAL_REVIEW.md](./09_CRITICAL_REVIEW.md) §2.3 pre-registered: *"Redesign only if you add a second vector backend — then introduce a `VectorStore` trait at the boundary."* The trigger has now fired. Qdrant stays the default until memvid survives the spike + a few weeks of dogfooding.

**One boundary must not move:** the Markdown vault stays canonical. A binary `.mv2` file is *less* inspectable than plain Markdown, and human-readable memory is eris's sovereignty story. Memvid may only ever hold **derived, rebuildable data** (the semantic index, episodic stream) — exactly the role Qdrant plays today. If the index dies, boot ingest rebuilds it from the vault. That property is non-negotiable.

---

## 2. The promotion tiers: yes, overengineered — here's the honest shape

### What exists (inventory)

- `EphemeralTier`: `Session → Scratch → Promote` ladder (`memory/types.rs:24–58`), only `Promote` eligible for `memory:commit_all`.
- `promotion_score: f64` per entry, incremented by mentions/staging, **decayed every daemon tick** (`ephemeral.rs:447–449`).
- Promotion when score ≥ threshold (`ephemeral.rs:451–468`); **demotion** when score < 0.5× the threshold below (`470–486`).
- Config surface: **6+ knobs** — 3 TTLs, 2 thresholds, decay-per-tick (`config.rs:715–731`).
- Coupling cost: `promotion_suppressed_during_step: Arc<AtomicBool>` shared between orchestrator and snapshot daemon — **the single exception to your own no-shared-mutable-state law**, existing solely so tier evaluation doesn't interleave with `step()`.
- Contradiction flag blocking auto-promote (`ephemeral.rs:33`).
- Plus `turn_end.rs` mention-promotion and 5 memory tools (`stage`, `staged_list`, `commit`, `commit_all`, `query`).

### Second-order observation

The ladder simulates a forgetting curve — score accumulation, decay, demotion — for a system with **one user, no tuning data, and no observer**. Nobody has ever looked at a Scratch-tier entry and its score and made a decision based on it. The parameters (thresholds, decay rate) were chosen by intuition and have never been validated, because there is nothing to validate them against. Meanwhile the machinery costs: 6 config knobs, a mutation tick, the atomic-flag coupling with the orchestrator, and a mental model with 3×2 transition edges.

What the system *functionally* needs is two states, not three tiers:

1. **Staged** — working memory with a TTL; visible via `memory:staged_list`; the LLM or user can commit it.
2. **Committed** — Markdown in the vault, indexed into the vector store.

The guard function you built the ladder for ("don't let junk auto-flow into the vault") is better served by what you already have: **explicit commit** (`memory:commit`), the **turn-end mention heuristic**, and the contradiction flag. Write-time curation by score was a third mechanism guarding a door that two mechanisms already guard.

### Proposed simplification (deletion-driven)

| Delete | Keep |
|---|---|
| `Scratch` tier, `EphemeralTier::next/prev/index`, promotion ladder | One `staged` state with TTL |
| `promotion_score`, decay-per-tick, demotion logic (`evaluate_promotions_and_decay` shrinks to TTL housekeeping or disappears — moka handles TTL natively) | Contradiction flag (cheap, useful) |
| 6 config knobs → 1 (`staged_ttl_secs`) | `memory:stage/commit/commit_all/staged_list/query` tools (unchanged surface for the LLM) |
| **`promotion_suppressed_during_step` AtomicBool** — the whole reason it exists goes away | Snapshot daemon, persistence only (bincode snapshots) |
| `commit_all`'s Promote-only eligibility → "all staged, minus contradicted" | `turn_end.rs` mention handling (simplify: mention refreshes TTL instead of incrementing a score) |

Net effect: ~200–300 lines and a concurrency exception deleted, zero capability lost, one less thing to document for contributors. The LLM-facing tool contract doesn't change, so no prompt/descriptor churn.

### The deeper move (this is where memvid re-enters)

The ladder was a **write-time gate**: curate before anything reaches long-term memory. The alternative model — which memvid's append-only frame design happens to fit exactly — is **read-time ranking**:

- Append cheap episodic records (turn digests, tool outcomes, mentions) to an append-only store *without* curation. Storage is nearly free; a single `.mv2` file with a time index is built for this.
- Do the filtering at **retrieval**: score = semantic similarity × recency × (optionally BM25), which is what `search_memory_query` and turn-start prefetch already compute. Junk that never matches any query costs nothing and harms nobody.
- Explicit vault commits remain the *curated, human-readable* layer — unchanged, and still the only thing a human is expected to read.

That inverts the guard from "decide at write time what will matter" (unknowable) to "decide at read time what matters now" (exactly what embeddings are for). The promote ladder was an attempt to solve the first problem; deleting it and leaning on retrieval solves the second one instead.

---

## 3. Phased plan

Sequencing note: **none of this belongs before the OSS launch M1.** Phase A is a pure code deletion and could ship anytime; B–D are post-launch (M2+). Do not let a new memory backend delay the installer.

| Phase | What | Risk | Gate |
|---|---|---|---|
| **A — Tier simplification** | Delete ladder/score/decay/demotion + AtomicBool coupling as per §2. Independent of memvid entirely. | Low (deletion + existing tests) | Anytime; nice pre-launch code-health win |
| **B — Spike memvid** | 1–2 day throwaway: external-vectors question, 768-dim HNSW, kill-9 durability, ingest the real vault, measure size/latency vs Qdrant. | None (throwaway) | Spike answers both §1 questions positively, else stop here |
| **C — `VectorStore` trait** | Extract the narrow boundary from `semantic.rs` (the audit's split seams: qdrant client / ingest / query make this natural — deep review §6). Qdrant impl = default. `document_store.rs` adopts the same trait (kills the duplicated Qdrant lifecycle + the second hardcoded 768). | Medium (refactor, but pre-registered in 09 §2.3) | Tests green; no behavior change with Qdrant backend |
| **D — Memvid backend** | `memory_backend = "qdrant" \| "memvid"` in config, memvid marked experimental. Win to advertise: **zero-daemon install** — the semantic index becomes one file next to the vault. Migration = boot re-ingest from vault (already exists; the index is derived data). | Medium | Weeks of dogfooding on your own vault before it's mentioned in user docs |
| **E — Episodic append stream** (optional, later) | Turn digests appended to the memvid timeline; prefetch queries it with recency×similarity. This is the read-time-ranking model of §2 realized. | Design work | Only after D is trusted; also a great essay for the content flywheel (§4 of [12](./12_META_STRATEGY.md)) |

### What not to do

- **Do not replace the Markdown vault with `.mv2`** — canonical memory stays human-readable plain text, or the sovereignty pitch dies.
- **Do not run memvid's bundled ONNX embedders** alongside llama-server — one inference owner (fails the Phase B gate if unavoidable).
- **Do not keep both backends forever.** The trait is a migration corridor, not a feature matrix. After D proves out (or fails), pick one and delete the other within a release or two — you already know from the Ollama/llama.cpp pair what a permanent dual backend costs.
- **Do not tune the ladder instead of deleting it.** Adjusting thresholds on unobserved machinery is effort spent making the overengineering more precise.
