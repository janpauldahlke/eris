---
id: skill-authoring-meta
title: How to create or update skills
priority: mandatory
triggers: skills:list,skills:read,skills:create,vault:write,vault:search
---
Use this skill when you need to create, refine, or de-duplicate another skill.

## When to create a new skill

Create a new skill only if at least one is true:
1. The workflow is multi-step and likely to recur.
2. A brittle tool needed retries or a non-obvious recovery path.
3. The user corrected the approach and the correction is reusable.
4. Existing skills do not already cover the behavior.

## When NOT to create a new skill

Do not create a new skill if:
1. The task was one-off or trivial.
2. The behavior is already covered by an existing skill and only needs a small update.
3. The procedure has no clear trigger conditions.

## Required format

Write skill files to:
- `10_Topology/skills/<id>.md`

Use this exact structure:

```markdown
---
id: <kebab-case-id>
title: <short clear title>
priority: mandatory|conditional
triggers: tool:a,tool:b,phrase:c
---
<one-line purpose>

## Procedure
1. ...
2. ...
3. ...
```

Rules:
- `id` must be stable kebab-case and unique.
- `triggers` must include relevant tool names where possible.
- Keep procedures concise and operational.
- No secrets, no credentials, no private tokens.

## Update vs create decision

Before writing:
1. Run `skills:list`.
2. If similar skill exists, run `skills:read` on it.
3. If behavior overlaps strongly, patch/update existing skill instead of creating a new one.
4. Create a new skill only for clearly distinct workflows.

## Safety constraints

1. Never encode fabricated contact info, placeholder addresses, or guessed credentials.
2. Never include user secrets from memory or config.
3. Prefer deterministic steps over vague advice.

## Verification checklist

After writing or updating a skill:
1. Confirm file path and id match.
2. Re-open with `skills:read` and verify parsed fields.
3. Ensure `triggers` align with actual tools.
4. Ensure procedure is short enough for bounded JIT injection.
