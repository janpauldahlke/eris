29/03/2026 today we begin with an empty project

- [x] (S00): establish foundation, error taxonomy, and telemetry
- [x] (S00.1): implement async router, cancellation token, and extended error taxonomy
- [x] git(S00.2): implement hierarchical config resolution via figment
- [x] (S00.3): implement async identity bootstrap and physical workspace seal"

30/03/2026

- [x] (S01): implement asynchronous ollama engine and wiremock test harness
- [x] (S01.1): integrate reasoning router FSM and token firewall
- [x] (S02): Subsystem Tier 1 is postponed, we need S04 (The Orchestrator) to make this work
- [x] (S02.1): implement tier 2 ephemeral memory with moka and bincode snapshot daemon
- [x] (S03): implement tool trait boundary and gatekeeper path firewall
- [x] (S04): define orchestrator shell and agent state schemas
- [x] (S04.1): implement context assembler and llm response router
- [x] (S04.2): implement orchestrator cognitive loop and recovery matrix
- [x] (S04.3): implement heartbeat monitor, async guillotine, and s02 condensation loop

31/03/2026

- [x] (S05): implement semantic guards, schemars validation, and gatekeeper state routing
- [x] (REFACTORING): refactor: enforce idiomatic module boundaries for engine and orchestrator
- [x] (S06): implement physical vault tools (read, write, list) and path firewalls
- [x] (S06.1): implement memory and system tool stubs with strict schema generation
- [x] (S06.2): implement temporal memory agenda queue and heartbeat consumer
- [x] (S06.3): implement web fetcher with panic-safe truncation and graceful 404 routing
- [x] (S07): implement TUI raw
- [x] (S07.1): implement neural link channels and wire orchestrator loop to tui app
- [x] (S08): implement pre-flight verification and non-blocking black box telemetry
- [x] (REFACTORING) (S06.x): replace hardcoded vault read limit with dynamic config ratio
- [x] (S09): ignation wizard for fresh system start
- [x] (S10): Semantic Memory / Vector Brain implemented via Qdrant
- [x] (S10.1) Vault Auto-Routing
- [x] Patch (S10.1): The Smart Router / YAML Interception
- [x] Patch (S11): The Cockpit Polish
- [x] fix(S04.\_1): implement context assembler to dynamically inject identity and JSON schemas into system prompt
- [x] Patch (S04.\_) JSON Lockdown
- [x] fix (S07): scrollabel viewport
- [x] fix: clippy issues
- [x] patch (s04.\_) add status field to JSON structure
- [x] fix (S07): scrollabel viewports inside the windows II
- [x] REVIEW (Sxx): better tracing, connect the dots, even better UI,
- [x] REVIEW (S01.1): move to pre-LLM routing approach
- [x] ENHANCEMENT (S05):add more semantics to tools catalogue, lower similarity for noemic-embed and acutal model
- [x] patch (S02): connect ephemeral and physical layer to make it persist
- [x] patch (S02): better staging and more instructive fields

01/04/2026

- [x] refactor (s04): CHANGES ephemeral and staging process
- [x] refactor (S08_Preflight): harden init and tear down of peripheral systems // user info for this progress
- [x] IMPROVEMENT (S03): introduce ticket like workflow for tools to avoid duplications in calls
- [x] REFACTOR Phase A (S6.01): completly rework web:search improved chuking // in memory // llm lense
- [x] REFACTOR Phase B (S6.01): completly rework web:search // making ingest a util
- [x] IMPROVEMENT (S07_TUI): better status indictors and guards to improve message queue
- [x] IMPROVEMENT (S05): use \*.toml as tools example to seperate rust code and description -> /tool_specs created
- [x] REFACTOR (S04): resolve technical debth, breaking down core.s and create a dedicated /loop to distinct policies and code
- [x] IMPROVEMENT (S04 / memory): orchestrator tools-first JSON handling // commit_all vs semantic skew fix
- [x] Improvement (S09): add user name to ignation process
- [x] feature (S06): add clock and alarm tool
- [x] REFACTORING(S06): Tier1+Tier2 tool prompt + routing fixes
- [x] IMPROVEMENT (S08 / telemetry): structured pre-LLM routing logs
- [x] fix (s02): give semantic brain retry attempts and make app fail if it istn ready
- [x] improvement (s02): harden memory query recall on ingest also uses title and tags

02/04/2026

- [x] REFACTOR (S05): revert top-k tool usage approach, it is harmfull
- [x] IMPROVEMENT (S0x): change context handlign to dynamic cts slider approach, condensatoi will be injected
- [x] IMPROVEMENT (S08TUI): tweak and improve UI, sync states in status as SSOT
- [x] feature (S06_tool): add remove action to agenda tool
- [x] feature (S06_tool): linking agenda and reminder to trigger with content / DONE vs SNOOZE
- [x] feature (S06_http_middleware): API tool below surface
- [x] feature (S06_tool): weather:forecast surface tool
- [x] feature (S06_tool): knowledge gather wiki tool
- [x] DOCS adding TODO about new yield/sleep mode
- [x] feature (S0x util): create fs dir and file watcher
- [x] improve and unify path handling for workspace ignation

03/04/2026

- [x] fix (S06): remind me at tool, deterministic follow up complete on user interaction
- [x] refactor (S00): align num_ctx usage across project, remove unused
- [x] feature (S0x): ship optimize_context feature flag and aplly it for tool returns to thin out context_slider
- [x] feature(S05): context diet for tool calls in context_slider winwdow
- [x] improvement (S05): overall tool outout for user, better format, better follow up JIT instruction
- [x] improvement (S0x): harden semantis for memorie and web fetch to harden boundaries
- [x] improvement (S03): improve memory retrival analysis
- [x] META curate docs and update architecture overview
- [x] BUG (JSONS_CONTRACT): enforce jsons contract and message_to_user contract
- [x] CHANGE (S4.03): gate heartbeat behind feture flag, disable it
- [x] allow model to see architecture docs in 99_USER_UPLOADED
- [x] better json repair advice for recovery

06/04/2026

- [x] integrate gws-builder crate
- [x] feature(S07TOOLS):create mail:send, mail:read, mail:write actions
- [x] test mail feature after making myself a google workspace admin

07/04/2026

- [x] improvement (s02): maps tags to vault spaces
- [x] fix: better startup to prevent multiple ollama instances that kill RAM
- [x] improved mail, mail:delete, mail:check, mail:move

08/04/2026

- [x] feature: agent facing tool compendium to prevent systemprompt bloat
- [x] feature ZETTELKASTEN - complete redesign of the vault structure and persistnce layer
  - do a each turn check on prending memories to raise score, daemon will deacy is
  - move promotion/decay to idle ticks
  - finding middle ground in config
  - updated architecture
  - clean up config
- [x] breakdown orchestratror, clean code!
- [x] better user facing message after failed recovery
- [x] better telemetry in TUI, shortening
- [x] undo: remove rolling_summary_condensation write to memory sidefect
- [x] slimmer context_slider by removing RECOVERY overhead and keeping context slider lean

10/04/2026

- [x] feature: decouple tui by creaging a presentation layer pahse 1 and 2
- [x] decouple chat session and router
- [x] web ui 1st iteration / ez and cheap
- [x] handle shutdown from web ui + browser close
  - minor tweak FCP json repair
  - 100vh max, and scrollables
- [x] fix mail send 3 times issue
- [] feature: big content lense
  - remove web:fetch
  - testing on large files

11/04/2026

- [] feature: big content lense
