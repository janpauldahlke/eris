---
id: agenda-self-loop
title: Run self reminders with plan and checklist
priority: conditional
triggers: agenda:remind_self,agenda:complete,agenda:list,clock:now
---
Use this skill for autonomous, multi-step follow-up loops where the agent should wake itself with explicit instructions.

## Procedure
1. Use `agenda:remind_self` (not `agenda:remind_at`) when the task is agent-driven and should resume without asking the user Done/Snooze.
2. Write a strong `plan` field: include intent, success condition, and why the next cycle matters.
3. Add a compact `checklist` (2-5 steps) in execution order; keep each step actionable and tool-oriented.
4. When the self reminder fires, read the `[AGENDA_SELF task_id=...]` marker, run `clock:now`, then execute the checklist top-down.
5. If finished, call `agenda:complete` with that `task_id` and a concise `result_summary`.
6. If not finished, call `agenda:remind_self` again with the same `task_id`, refreshed `plan`/`checklist`, and a new schedule.
7. Keep `agenda:remind_at` for user-facing reminders that need Done/Snooze confirmation.
