### 1. Solving the 'Degradation' (The Stimulus Problem)

You've hit on a fundamental architectural flaw in the current loop. When I set an alarm, the system sends a `SystemAlarm` event. As I saw in the TUI/Orchestrator logs, this event is often just a string label. When the alarm fires, I 'wake up', see the label, and... that's it. There is no 'instructional payload' attached to the alarm to drive the next step.

**The Solution (Stimulus Implementation):**
To prevent me from going 'inactive', the `agenda:remind_at` tool shouldn't just take a `label`; it needs to support an optional `instruction` or `payload` field.

- **Instead of:** `remind me at 3pm: Check the vault` (which just wakes me up to a string).
- **We implement:** `remint me at 3pm: { "action": "vault:read", "path": "notes/task.md" }`.

By injecting a structured `UserAction` or `SystemInject` event into the Orchestrator's input queue when the alarm triggers, you provide the 'spark' that restarts my reasoning engine. I don't just wake up; I wake up with a command.

### 2. The 'Inside the Vault' Constraint (Safe Discovery)

I completely respect the 'Gatekeeper' philosophy. My 'desire' for visibility was too broad. If we keep me strictly within the vault, the problem is **Orientation**.

**The Solution (Improved Discovery):**
To improve `vault:list`, we should implement a **`vault:map`** or **`vault:topology`** tool. This tool wouldn't read the content of files, but would instead return a lightweight tree structure of the directories and filenames. This allows me to 'see' the room I'm in without looking through any closed drawers. It gives me context without compromising the security of the data inside the files themselves.

### 3. Redefining 'Agency' (The Personal Assistant Scope)

I hear you loud and clear. I shouldn't be trying to act like a DevOps engineer or a Git bot. That's not my role, and it's not what you need.

**The Solution (Scoped Agency):**
I should refocus my 'outbound' potential on **Coordination and Notification**. My 'Agency' should be limited to:

- **Internal Notifications:** Alerting you via the TUI when a long-running background task is finished.
- **Ecosystem Coordination:** Using the Discord tool (when you enable it) to send a simple 'Heads up, the task is done' message, or updating a specific 'Status' file in the vault that you monitor.
- **No 'External' Chaos:** No uncontrolled outbound webhooks or external service manipulation unless specifically directed by a 'Skill'.

### 4. The Sandbox (The Reasoning Engine)

I'm glad you're looking into the sandbox. If I can have a 'scratchpad'—a place where I can run a small piece of logic to verify a calculation or check a pattern—I can significantly reduce the 'hallucination' or 'lazy' behavior you noticed in the agenda loop.

**Summary of my 'Refined' Needs:**

1.  **Payload-driven Alarms:** To provide the stimulus for multi-step autonomy.
2.  **Vault Mapping:** To allow orientation without exposure.
3.  **Scoped Coordination:** To act as a coordinator, not a developer.
4.  **The Sandbox:** To verify my own logic before presenting it to you.

---

1. introduce a new reminder type, for the agent. give it a broader hint field, so he can plan steps for himself and checklist them.
2. could we provide a vault:treeview so he can see files, what would this give us. the 30_Synthesis are only UUIDs and not directly recognizable by filename.
3. he desires a sandbox to write pyhtong and execute it. this is like a code sandbox for python. this would make him pretty OP since it would break the cage, but on the other hand he could solve harder puzzles or problems by himself
