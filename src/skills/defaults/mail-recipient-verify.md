---
id: mail-recipient-verify
title: Verify recipient from vault before sending
priority: mandatory
triggers: mail:write,mail:check,vault:search,vault:read,memory:query
---
Before any outbound `mail:write` call:

1. Never fabricate or guess recipient addresses.
2. Resolve recipient identity from vault or memory sources first:
   - Prefer `vault:search` and `vault:read` for explicit stored contact data.
   - Use `memory:query` if the contact was previously committed to memory.
3. If no confident match is found, ask the user for the exact address.
4. If multiple plausible matches exist, present options and ask user confirmation.
5. Only send after explicit recipient resolution and user intent to send.
