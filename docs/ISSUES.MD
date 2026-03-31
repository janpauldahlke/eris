. The Sharp Critique: What is the Stack Missing?

You asked for a ruthless assessment of our current stack. Here is exactly where the hull is going to crack when you put it under sustained pressure:

    Fatal Flaw A: Context Collapse (The OOM Trap). We built the S06.x guillotine to stop ERIS from reading files that are too big. But we have no protection against a long conversation. If you chat with Adam for four hours, the messages array in your Orchestrator state will quietly exceed 16,384 tokens. The next time you hit enter, the Ollama API will throw a hard 500 Context Overflow error, and the agent will instantly die. Addition needed: An "Episodic Condenser" that runs in the background, summarizing the oldest messages in the chat array into a compressed memory block, keeping the active token count permanently under 75% capacity.

    Fatal Flaw B: The UX Freeze (No Streaming). Because we force the LLM to output strict JSON ({"thought": "...", "tool_calls": [...]}), your reqwest client is waiting for the final } before it hands the payload to serde_json. If Adam decides to write a 2,000-word file using vault:write, the TUI will sit completely frozen for 15 seconds while the tokens generate in the background. Addition needed: A partial JSON streaming parser that updates the TUI viewport token-by-token so you aren't staring at a dead screen.

    Missing Capability: Parallel Execution. Right now, if Adam needs to search the web for three different things, he does it sequentially: Search -> Wait -> Search -> Wait. Standard APIs (like OpenAI) support parallel tool calling. Qwen is smart enough to output an array of three tools at once. We need to refactor the Gatekeeper to use tokio::join! to execute tool arrays simultaneously.
