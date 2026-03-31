# ERIS (FCP) - Drydock Ledger & TODOs

## [CURRENT STATUS]

- **Core Engine:** ONLINE (S00 - S04.3)
- **Gatekeeper/Tools:** ONLINE (S05 - S06.3)
  - Vault I/O, Agenda Queue, Web Fetcher fully operational.
- **Terminal UI:** ONLINE (S07 - S07.1)
  - Asynchronous `ratatui` HUD successfully wired to the Orchestrator telemetry.
- **Current Defect:** _Cognitive Schism._ The LLM is hallucinating conversational text instead of strict JSON because the Core Identity prompt is not being injected into the Context Assembler.

## [PENDING BLOCKS (NEXT ACTIONS)]

### 1. Fix the Identity Injection (Patch S04.x)

- **Goal:** Wire the `00_Core/Identity.md` file (or a hardcoded Pre-Identity system prompt) into the `ContextAssembler` to force the LLM into strict JSON tool-calling compliance and stop the S04.2 Recovery Matrix from tripping.
- lets not hardware this but find mechanism on init, where the TUI asks how to name it and what model to chose
  - we could to a `ollama ps` to seen all models
- then onInit we populate the vault and provide a default persona for now
- later we make the user a section of settings, where he can curate this for him self
- see (First Boot Wizard) BLOCK

### 2. Block S08: Semantic Memory (The Vector Brain)

- **Goal:** Breathe life into the `memory:query` and `memory:commit` stubs.
- **Tasks:** - Integrate `qdrant-client` into the Rust binary.
  - Wire `nomic-embed-text` via Ollama API to generate vector embeddings.
  - Implement RAG (Retrieval-Augmented Generation) so ERIS can search her own Vault.

### 3. Block S09: Telemetry & Pre-Flight (The Black Box)

- **Goal:** Implement the physical deployment profile and post-mortem logging.
- **Tasks:**
  - Add `tracing-appender` to write rotated logs to `.fcp/logs/` (bypassing the TUI UI).
  - Implement the Pre-Flight Matrix: Ping `localhost:6334` (Qdrant) and `localhost:11434` (Ollama) before booting the TUI. Abort with clear instructions if daemons are down.
  - Update `Cargo.toml` `[profile.release]` with `lto = "fat"` and `panic = "abort"`.
  - Build the `fcp log` CLI bypass to read the black box.

### 4. Block S07.2: The Ignition Sequence (First Boot Wizard)

- **Goal:** Build the interactive setup wizard for new workspaces.
- **Tasks:** - Use `inquire` or `dialoguer` to ask the user to select an Ollama model on first run.
  - Write the selection to `config.toml` and drop the `.fcp_seal` to initialize the workspace.

### 5. clean up UI

**findings**

- show cursor pos
- show prev prompt and answer
- respect grid end - text overflows out of view // scalable?
- maybe a new section to display ERIS errors?
- /exit should exit and close program

### questions

- how to we prevent that each prompt will be checkd for tools, in the previous iterations, i regEXP for certain keyworkds, but how to distinct search fdor vault by search use web_search, we need to harden this somewhat

### the future

- we should align the lenght of one block with the max context-window \* 0.75
- by this we shoudl achieve that each file in markdown maps the context size
- is there a mechanisam in place to move files into the proper folder, like episodic, sematinc, user, ..... we need to check on the vault structure again

### 1.5 Patch S06.x: The Dynamic Context Guillotine

- **Goal:** Remove the hardcoded 3000-token limit in `vault:read`.
- **Tasks:** Add `llm_context_window` and `vault_read_ratio` to `AppConfig`. Inject the dynamically calculated limit into the tool upon initialization.
