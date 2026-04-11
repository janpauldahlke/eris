## Eris Architectural Directive: The Autonomous Gardener Subsystem

### I. The Problem Space

##### Thesis:

A purely reactive Large Language Model (LLM) orchestration layer is structurally stagnant. It requires explicit user invocation to process information, turning the agent into a passive query-response mechanism rather than a knowledge curator.

##### Antithesis:

Implementing biological or psychological autonomy (as proposed in The Synthetic Mind) introduces fatal engineering vulnerabilities. Time-based memory decay (ACT-R), strict capacity limits (Miller’s Limit), and continuous continuous emotional state vectors (PAD space) require arbitrary floating-point tuning. When applied to a local vault utility, this biological mimicry results in context starvation, hallucination loops, and irreversible semantic degradation of the user's primary data structure.

##### Synthesis:

The system requires autonomy driven by mathematical necessity, not simulated psychology. We must decouple the continuous execution loop from the text-generation engine. The solution is Topological Determinism: a purely functional Rust state machine that calculates structural tension within the vault graph and invokes the LLM strictly to resolve that tension within a confined topological sandbox.

### II. Telos

The ultimate objective is a self-organizing, self-healing Markdown knowledge graph. The Gardener subsystem acts as an autonomous conduit between raw, fragmented ingestion (Ephemeral Memory) and structured, long-term recall (Semantic Memory). The telos is strictly defined by three invariants:

    Deterministic Pressure: Autonomous action is triggered solely by measurable structural deficits in the graph, never by chronological time.

    Data Immutability: The agent may never alter core vault axioms or topological scaffolding.

    Compute Conservation: The LLM is isolated behind a strict gateway and invoked only when topological entropy exceeds the compute threshold.

### III. The Doing: Implementation Patterns and Flows

#### Pattern 1: The Dual-Process State Machine

The architecture enforces a hard boundary between System 1 (Rust/Fast) and System 2 (LLM/Slow).

    System 1 (The Daemon): A lightweight background tick operating independently of the TUI. It holds no text and builds no context. It performs continuous, low-latency graph queries (counting nodes, reading tags, measuring cache depth).

    System 2 (The Actuator): The Orchestrator and LLM. It remains dormant until System 1 pushes a synthetic AutonomousTrigger event into the primary event loop.

#### Pattern 2: Topological Entropy (The Drive Engine)

We discard biological drives (e.g., curiosity, boredom). The system operates on Topological Entropy. Entropy is calculated as a deterministic integer score summing unresolved state variables:

    Count of unmerged web artifacts residing in the moka staging cache.

    Count of orphaned nodes (files lacking structural links).

    Count of unresolved tags (e.g., #needs_review).
    When this deterministic sum crosses a parameterized threshold, System 1 demands an actuation cycle. The system does not act because time has passed; it acts because the workspace is chaotic.

#### Pattern 3: The Saturation Pre-Filters (Bounding the Context)

To prevent the LLM from hallucinating due to context starvation or entering infinite rewrite loops, we replace rigid working memory limits with saturation filters.

    Context Saturation: When System 1 triggers an actuation, the ContextAssembler pulls target nodes. Before appending a node to the LLM view, it evaluates its semantic redundancy (via embedding cosine distance) against the current prompt centroid. If the new node is semantically identical to existing context (Saturation > 0.85), it is dropped. This guarantees dense, non-redundant context windows.

    Action Saturation: A node successfully synthesized by the Gardener receives an AgenticCooldown metadata tag (or ephemeral cache flag). Saturated nodes are mathematically invisible to the entropy calculation for n ticks, forcibly breaking autonomous looping and shifting the agent's attention to neglected topologies.

#### Pattern 4: The Spatial Constraint (The Sandbox Flow)

To prevent autonomous degradation of the primary knowledge base, the Gardener’s execution routing is spatially segregated.

    Read-Only Scope: The agent is granted read access to the entire Qdrant semantic space and the filesystem, specifically prioritizing 00_Invariants and 10_Topology to maintain structural alignment.

    Write-Only Scope: Tool schemas available during an autonomous tick are strictly hardcoded to target a dedicated workspace. The agent consumes from moka (the staging cache) and executes writes explicitly into 30_Synthesis (or a designated ephemeral output directory).

Execution Flow (The Gardener Cycle):

    Monitor: gardener_daemon ticks, calculating Topological Entropy via fast file/cache checks.

    Trigger: Entropy breaches threshold. Daemon injects UserAction::AutonomousTrigger to the orchestrator.

    Assemble: Orchestrator queries the highest-entropy targets. ContextAssembler hydrates the targets from Qdrant/Moka, applying the Context Saturation filter.

    Execute: Orchestrator bypasses the chat view, generating a hidden system prompt instructing the LLM to resolve the specific targets.

    Commit: LLM invokes the synthesize_node tool. Data is written to 30_Synthesis.

    Cooldown: Orchestrator applies Action Saturation flags to the processed targets.

    Reset: Entropy drops mathematically. The daemon returns to silent monitoring.

## Ppossible implementation plan

### Phase 1: Crate Topology & The Trait Boundary

The eris_gardener crate must be functionally pure. It calculates state and returns instructions; it does not perform physical I/O.

1. Create the Workspace Crate:
   Initialize crates/eris_gardener/.
   Dependencies: tokio, serde, parking_lot. Do not pull in reqwest, moka, qdrant-client, or nomic-embed.

2. Define the Inversion of Control:
   The gardener needs to read the vault state without knowing how the vault is stored. Define a strict trait boundary in eris_gardener:
   Rust

```
pub trait VaultTopologyView: Send + Sync {
fn count_staged_artifacts(&self) -> usize;
fn count_orphaned_nodes(&self) -> usize;
fn count_unresolved_tags(&self, tag: &str) -> usize;
fn get_node_embedding(&self, id: &str) -> Option<Vec<f32>>;
}
```

The main Eris Orchestrator will implement this trait, wrapping your moka cache and Qdrant queries.

### Phase 2: The Entropy Engine (System 1)

This module tracks the state variables and calculates the deterministic pressure.

1. The Entropy Struct:
   Create src/entropy.rs. Define the configurable thresholds and the evaluation logic.
   Rust

```
pub struct EntropyConfig {
pub trigger_threshold: u32,
pub weight_staged: u32,
pub weight_orphaned: u32,
}

pub fn evaluate*pressure(view: &dyn VaultTopologyView, config: &EntropyConfig) -> u32 {
let staged = view.count_staged_artifacts() as u32 * config.weight*staged;
let orphaned = view.count_orphaned_nodes() as u32 * config.weight_orphaned;
staged + orphaned
}
```

2. The State Machine:
   Create the GardenerState struct wrapped in an Arc<RwLock<>>. It holds the current entropy level, the timestamp of the last synthesis cycle, and the AgenticCooldown registry (a simple hash map of node IDs to UNIX expiry timestamps).

### Phase 3: The Saturation Filters

This module replaces ACT-R and Miller's limit. It governs what the LLM is allowed to see.

1. Context Redundancy:
   Create src/saturation.rs. Implement a fast cosine similarity function.
   Rust

```

pub fn is_redundant(new_embedding: &[f32], context_centroid: &[f32], threshold: f32) -> bool {
// Return true if cosine similarity > threshold (e.g., 0.85)
}

```

2. Target Selection:
   When entropy breaches the threshold, the crate must return an execution plan. Write a function select_targets(&self, view: &dyn VaultTopologyView) -> Vec<String> that selects high-entropy IDs (e.g., the oldest unmerged web artifacts) while filtering out anything currently in the cooldown registry.

### Phase 4: Integration (The Eris Actuator)

Move out of eris_gardener and into eris/orchestrator. You are wiring the logic into the physical engine.

1. The Adapter:
   Implement VaultTopologyView on a new GardenerAdapter struct within the orchestrator that holds references to EphemeralMemory and SemanticBrain.

2. The Daemon Loop:
   In orchestrator/loop/, write spawn_gardener_daemon.
   Rust

// Runs independently at a low frequency (e.g., 10 seconds)

```
tokio::spawn(async move {
let mut interval = tokio::time::interval(Duration::from\*secs(10));
loop {
tokio::select! {

- = interval.tick() => {
  let current*entropy = eris_gardener::entropy::evaluate_pressure(&adapter, &config);
    if current_entropy >= config.trigger_threshold {
        let targets = eris_gardener::select_targets(&adapter);
        let * = action_tx.send(UserAction::System(AutonomousTrigger(targets))).await;
      }
    }
    }
    }
  });
```

### Phase 5: The Sandbox Execution (System 2)

This modifies the main orchestration loop when the AutonomousTrigger is intercepted.

1.  Gated Context Assembly:
    Do not use the standard chat_stack. Build a shadow context:
    - Extract the markdown for the target IDs provided in the trigger.
    - Calculate their embeddings. Run them through eris_gardener::saturation::is_redundant against the active subset. Drop redundant nodes.
    - Assemble a static system prompt: "System Drive: Vault Maintenance. Synthesize the following fragmented context into cohesive markdown nodes."

2.  The Tool Restraint:
    During an autonomous tick, the Gatekeeper must block all standard tools. Only allow one tool: Tool::synthesize_node.
    Ensure the execution of synthesize_node hardcodes the destination path to 30_Synthesis/{uuid}.md. It must return a hard error if the LLM attempts to write to 00_Invariants or 10_Topology.

3.  Cooldown Application:
    Upon successful tool execution, the Orchestrator calls eris_gardener::register_cooldown(target_ids, duration). The original targets are cleared from moka, the new node is ingested into Qdrant, and the background daemon inherently calculates a lower entropy score on its next tick.
