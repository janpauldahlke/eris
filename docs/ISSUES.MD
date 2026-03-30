# S04 Execution Plan: The Orchestrator (Base)

## Step 2: The Orchestrator Boundary

We will build the struct that holds the initialized S01, S02, and S03 components. Do NOT implement the `step()` function.

**1. The Target (`src/orchestrator/mod.rs`)**

```rust
use crate::engine::LlmEngine;
use crate::tools::Gatekeeper;
use crate::memory::ephemeral::EphemeralMemory;
use crate::orchestrator::state::AgentState;
use std::sync::Arc;

pub struct Orchestrator<E: LlmEngine> {
    pub state: AgentState,
    pub engine: E,
    pub gatekeeper: Gatekeeper,
    pub ephemeral: Arc<EphemeralMemory>,

    // Bounds
    pub max_recovery_attempts: u8,
    pub max_tool_rounds: u8,
    pub condensation_threshold: f32,
    pub num_ctx: usize,

    // Live Loop State
    pub recovery_count: u8,
    pub tool_rounds: u8,
}

impl<E: LlmEngine> Orchestrator<E> {
    // Implement `new` constructor.
    // Initialize state to AgentState::Idle.
    // Initialize live loop state counters to 0.
}
```

**2. Failing Test First**
Write `test_orchestrator_initialization` in `src/orchestrator/mod.rs`.

- Create a mock `LlmEngine` (or use a dummy struct that implements the trait).
- Instantiate a dummy `Gatekeeper` and `EphemeralMemory`.
- Call `Orchestrator::new(...)`.
- Assert `state` is `Idle`, and `recovery_count` and `tool_rounds` are `0`.

**3. HARD STOP.** Implement the `new` constructor to make the test pass. Do not write `step()`.

```

```
