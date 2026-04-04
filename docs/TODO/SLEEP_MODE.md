The Architectural Fix: The Sleep State

check on GARDENER_MVP.md . this is prerequiste

You must introduce a fourth state strictly for background termination. Do not overload Idle. Idle means "Yield to keyboard." You need a state that means "Yield to scheduler."

I recommend adding Sleep (or Yield).

Here is exactly how you modify the JSON shape and the rules block to accommodate the Autonomous Gardener without breaking the chat interface:

1. The Updated JSON Shape
   JSON

"status": "Task|Reflect|Idle|Sleep"

2. The Updated Rules
   Plaintext

Status rules (follow exactly):

1. Reflect: when calling one or more tools this turn. tool_calls MUST be non-empty.
2. Task: internal continuation or planning with NO tools this turn. tool_calls MUST be [].
3. Idle: INTERACTIVE mode complete. Waiting for the human user. tool_calls MUST be []. message_to_user MUST be a non-empty reply.
4. Sleep: AUTONOMOUS mode complete. Background task is finished. tool_calls MUST be []. message_to_user MUST be null.
5. If you need tools, prefer Reflect. The runtime executes tool_calls whenever they are non-empty (before status), so do not mix Idle/Sleep with tools.
6. `Process` is accepted as an alias for Task.
7. If no tool is needed, NEVER choose Reflect.
8. Do NOT respond with status Task when tool_calls is [] AND message_to_user is null/empty. If you need tools, use Reflect. If you are interacting with a human, use Idle. If you are running a background task and are finished, use Sleep.

How the Rust State Machine Handles This

In your src/orchestrator/core.rs, you will add a new match arm to your process_llm_response logic.

    If status == Idle: The Orchestrator pushes the message_to_user to the TUI and transitions to AgentState::HaltAndAwaitInput. It physically blocks on the MPSC channel waiting for your keyboard.

    If status == Sleep: The Orchestrator does not push anything to the TUI. It simply clears the active execution context, drops the thread, and resets the background timer. It goes dark until the next cron tick or agenda trigger.

By separating Idle (human interactive) from Sleep (machine autonomous), you keep the LLM's context window clean and completely eliminate the "talking to a wall" hallucination vector.

**Related:** [GARDENER_MVP.md](./GARDENER_MVP.md) §B.3 notes how the current heartbeat / idle interrupt interacts with post-absence turns; fold that into Sleep/Yield rather than ad-hoc heartbeat patches.
