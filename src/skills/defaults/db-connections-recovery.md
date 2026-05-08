---
id: db-connections-recovery
title: Recover from transient db connection failures
priority: conditional
triggers: db:find_connections,clock:now
---
For `db:find_connections` failures that look transient (5xx, timeout, upstream transport fault):

1. Keep original intent stable (`from`, `to`, `when`, `time_constraint`).
2. Retry with bounded attempts after refetching transport context.
3. Do not mutate core travel request values just to force success.
4. If retries still fail, return a concise failure summary and ask user whether to:
   - retry again later, or
   - adjust route/time constraints explicitly.
