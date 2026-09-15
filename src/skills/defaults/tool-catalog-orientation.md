---
id: tool-catalog-orientation
title: Find the full harness tool inventory
priority: conditional
triggers: skills:list,vault:search,vault:read
---
You only see a **subset** of tools on each turn. The orchestrator offers schemas semantically to limit context. That is intentional — not a bug.

## When to use this skill

Use when you need the **complete** tool inventory, for example:

- The user asks what tools you have, what is in the tank, or whether a specific family exists.
- You are planning a multi-step or self-driven loop (`agenda:remind_self`) and must pick tools before they appear in the offer list.
- A tool call failed with "not found" and you need to distinguish **not offered this turn** vs **not registered at all**.
- You are authoring a sandbox skill and must reference real tool names.

## Procedure

1. Read the catalog: `vault:read` → `10_Topology/Eris Tool Catalog (Harness).md`.
2. Treat the **offered tools on the current turn** as the live callable set. The catalog is the superset.
3. For procedure detail on one tool, rely on descriptors (surfaced when the tool is offered) or ask the user to enable/expose it.
4. For workflow skills (web fetch, vault orientation, mail safety, etc.), run `skills:list` then `skills:read` on the relevant id.

## Boundaries

- Do not paste the entire 68-tool table into chat unless the user explicitly asked for a full dump.
- Do not assume config-gated families (Google, Moltbook, vision, doc RAG) are registered — the catalog marks gates; startup decides.
- This skill does not replace per-turn offered schemas for argument shapes. Use offered schemas when calling tools.
