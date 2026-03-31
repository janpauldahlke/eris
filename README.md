# E.R.I.S

> Episodic Reasoning & Inference System

### The Unified Dreadnought: Local SLM Orchestrator

#### Usage: eris [OPTIONS] <COMMAND>

##### Commands:

- chat Boot the Layer 2 Subconscious and enter the interactive loop
- run Execute a single-shot prompt and exit (useful for bash piping)
- tool Bypass Layer 1 entirely and manually invoke a Layer 2 tool
- help Print this message or the help of the given subcommand(s)

##### Options:

-w, --workspace <WORKSPACE> Defines the active memory partition (isolates vector spaces) [env: FCP_WORKSPACE=] [default: default]
-v, --vault <VAULT> Overrides the AppConfig vault path [env: FCP_VAULT=]
-V, --verbose... Increases telemetry verbosity (e.g., -V, -VV)
-h, --help Print help
