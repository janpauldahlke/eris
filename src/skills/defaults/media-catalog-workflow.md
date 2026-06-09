---
id: media-catalog-workflow
title: Media catalog — remember, recall, and display uploaded images
priority: conditional
triggers: vision:see,vision:display,media:catalog,media:meta
---
Use this skill when the user uploads an image and asks to remember it, recall a cataloged image, or display one inline.

## Two tiers

- **99_USER_UPLOADED/** — blob storage (images, audio, files). Content-addressed filenames `{sha256}.ext`. Never embedded in Qdrant.
- **40_MEDIA/{hash}/media.json** — semantic catalog cards. Embedded in Qdrant as text only.

## When to use which tool

| User intent | Tool |
|-------------|------|
| What is in this attached image? | `vision:see` |
| Remember / save this image | `vision:see` then `media:catalog` |
| Show / display a known image | `vision:display` + short prose from catalog |
| Add or fix notes/tags later | `media:meta` |

## Remember flow (user says "remember this" with an upload)

1. **`vision:see`** on the attached image path — get a description.
2. **`media:catalog`** with `relative_path` + `description` from step 1.
3. **Title:** you invent a short descriptive label (e.g. "Fish truck at Zen market"). **Do not** use the user's words ("remember this") as the title. Omit `title` when the description is enough — the tool derives one from the first sentence.
4. Optional: `tags`, `user_notes` if the user gave extra context.

## Rules

- Do **not** catalog on every `vision:see` — only when the user asks to remember.
- Do **not** stop at citing a vault path when the user asked to **show** an image — call `vision:display`.
- Pass exact `relative_path` from upload hints or `40_MEDIA` recall (`path` / `file_path` are accepted — gatekeeper maps them to `relative_path`).
