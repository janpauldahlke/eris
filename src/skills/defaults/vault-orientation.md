---
id: vault-orientation
title: Map first then drill into the vault
priority: conditional
triggers: vault:taglist,vault:search,vault:list,vault:read
---
Use this skill before searching for content in the vault when you do not yet have a strong keyword. Map first, then drill.

## Procedure
1. Start with `vault:taglist` (no args, or `top_k` and `prefix`) to see the actual frontmatter taxonomy under `30_Synthesis/` and where the gravity sits.
2. When a tag looks promising, call `vault:taglist {tag: "<tag>"}` to receive the synthesis-relative paths that carry it.
3. Read each note with `vault:read` using the returned path; do not re-search by keyword if you already have an exact path.
4. Reach for `vault:search` only when you need full-text matching across notes (a phrase or pattern), not when you can already pick a tag.
5. `vault:list` is for folder-by-folder navigation; do not use it to discover topics.

## Boundaries
- `vault:taglist` only covers `30_Synthesis/`. Other folders (`00_Invariants`, `10_Topology`, `20_Discourse`, `99_USER_UPLOADED`) do not use consistent frontmatter yet, so they are intentionally skipped.
- The taglist snapshot is built at chat startup and rebuilt automatically after any successful `vault:write` under `30_Synthesis/`. If you suspect external edits (Obsidian, git pull), pass `refresh: true`.
- Tag matching is case-insensitive, but tags are normalized to lowercase. Pass `prefix` for partial matches.
