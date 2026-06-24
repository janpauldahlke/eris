---
id: doc-summarize
title: Summarize uploaded documents using paginated reading and scratch notes
priority: conditional
triggers: doc:read,doc:query,doc:list
---
Multi-step summarization workflow for ingested documents.

IMPORTANT: Always use `doc:read` (sequential paginated reading) as the primary tool for summarization. Do NOT use `doc:query` (semantic search) as a substitute — it returns scattered fragments, not the full document flow needed for a proper summary.

## Assess scope

Call `doc:list` or use a known `doc_id`. Note `total_chunks` to gauge document size.

## Small documents (under ~20 chunks)

A single `doc:read` call with `count` equal to `total_chunks` covers the whole document. Summarize directly from the returned text.

## Large documents (20+ chunks)

You MUST page through the full document with `doc:read` in increments of 15. Do not skip this step or substitute `doc:query` — semantic search cannot replace sequential reading for summarization.

After each page, append running section notes to `20_Discourse/doc-notes/{source_name}.md` via `vault:write` (mode: append). Notes should capture:

- Key claims and arguments
- Document structure and section headings
- Important terminology and definitions
- Anything the user specifically asked about

This scratch file is essential for large documents because earlier pages will fall out of context as you continue reading.

## Synthesize

After the final page, read back notes from `20_Discourse/doc-notes/` via `vault:read` and produce a structured summary:

1. **Purpose/thesis** — what the document is about
2. **Key sections** — structural overview
3. **Main findings/arguments** — the substance
4. **Notable details** — anything surprising or user-relevant

## Adapt to document type

- **Academic papers**: anchor on abstract + conclusion first, then fill in methodology and results
- **Contracts/legal**: clause-by-clause attention; flag obligations, deadlines, penalties
- **Technical manuals**: section-oriented; focus on procedures, warnings, specifications
- **Reports/slide decks**: executive summary first, then supporting data

## Update catalog (optional)

If the summary is useful, offer to update the `40_MEDIA` card description via `media:meta` for better future discovery through `memory:query`.
