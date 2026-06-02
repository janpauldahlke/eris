# AI SHOULD IGNORE THIS DOC!!!

## Backend Selection

Eris supports two LLM backends:

- **Ollama** (default): Manages models via Ollama's API. Easiest setup.
- **llama.cpp**: Direct GGUF model inference via llama-server. More control, GBNF grammar enforcement for structured output.

See [LLAMA_CPP_SETUP.md](LLAMA_CPP_SETUP.md) for llama.cpp installation and configuration.

Backend is selected during first-run ignition (`eris chat` in a fresh vault directory) or by setting `llm_backend` in `.fcp/config.toml`.

---

# THE SNIPER PROTOCOL (Operator Manual)

**The Prime Directive:** Never let the IDE auto-pilot a whole file. Build by the function, test by the function. The moment a test passes, the chat window is contaminated with dead context.

### 0. The Shipyard Ledger (Pre-Flight)

Before starting a session, ensure your root directory contains these operational ledgers:

- `state.md`: The macro-ledger. Tracks which Architectural Blocks (S00-S08) are complete.
- `GLOB.md`: The Sonar Log. Your persistent scratchpad for tech debt, architectural shifts, and active micro-roadmaps.
- _(Note: The AI's rules live strictly in `.cursorrules`. You enforce the rules below)._

### Phase 1: The Roadmap (The Architect)

- **The State:** Start a brand new chat window. Ensure you are using the heavy, high-logic model (e.g., GPT-4o, Claude 3.5 Sonnet, or Gemini Pro).
- **The Prompt:** `@docs/architecture/[target_block].md` _"Read the target spec. Generate a detailed, step-by-step TODO roadmap for this specific block. Define the exact Rust traits, function signatures, and the failing unit tests we need to write first. Stop and wait for my approval."_
- **The Action:** Review the micro-roadmap. If the logic is mathematically sound, approve it and copy it into your `GLOB.md` scratchpad.

### Phase 2: Test-Driven Execution (The Welder)

- **The State:** Switch the Cursor chat model to a fast, cheap model (e.g., Haiku, Flash) to save tokens and increase speed.
- **The Prompt:** _"Execute Step 1 from the roadmap. Write the failing test, then write the implementation to make it pass. Do not proceed to Step 2."_
- **The Human Circuit Breaker:** You run `cargo test`.
  - If the compiler screams about a simple missing `&` or `mut`, **fix it yourself**. Do not burn API quota on basic syntax.
  - If the error is an architectural deadlock, paste the 15-line trace into the chat: _"Fix this specific compiler error. Do not rewrite the whole file."_

### Phase 3: The Context Guillotine (Cost & Sanity Control)

- **The Trigger:** The moment Step 1 compiles and the test passes.
- **The Action:** WIPE THE CHAT WINDOW IMMEDIATELY. Start a brand new chat. Keep the fast model active.
- **The Next Prompt:** `@src/your_new_file.rs` _"We are on Step 2 of the roadmap. Write the failing test and implementation for [Next Function]."_

### Phase 4: The Merge & Reset (Version Control)

- **The Trigger:** The entire Architectural Block compiles and tests pass.
- **The Action:** **YOU** run `git add` and `git commit`. The AI is physically forbidden from touching your version control history.
- **The Reset:** Switch back to the heavy Architect model. Start a new chat.
- **The Prompt:** `@src/target_file.rs` _"Summarize what we built in this file. Provide the exact markdown text to update `state.md` with our new progress. Log any tech debt for `GLOB.md`."_
- **The Close:** Overwrite `state.md`. Wipe the chat window. Begin Phase 1 for the next block.

---

> @state.md @docs/architecture/S03_The Vault.md

> "Read the state ledger to understand where this subsystem fits into the overarching E.R.I.S. architecture. Then, read the target spec. Generate a strict, step-by-step TODO roadmap for the S00 target block only. Define the Rust traits, function signatures, and failing unit tests we need to write first. Do not write the code yet. Stop and wait for my approval."
