
#fcp

1. Objective
    
    Construct the visual manifestation of the Orchestrator. The Terminal User Interface (TUI) provides real-time state visualization, hardware telemetry, ASCII-based personality, and chat rendering without blocking the asynchronous execution of Layer 1 (Inference) or Layer 2 (State Machine).
    
2. Architecture & Design Rules
    

A. The Rendering Engine (Zero-Blocking I/O)

The TUI must never steal compute from the inference engine or freeze during a long Qdrant retrieval.

- **Tooling:** `ratatui` (rendering) paired with `crossterm` (input handling).
    
- **Concurrency:** The UI runs on an isolated `tokio` task.
    
- **Communication:** Connected to the Subconscious Orchestrator strictly via asynchronous `tokio::sync::mpsc` channels.
    
- **Event Architecture:** The loop processes three event types:
    
    - `Event::Tick`: Triggers a high-frequency screen redraw.
        
    - `Event::Input(KeyEvent)`: Captures user keystrokes.
        
    - `Event::StateUpdate(Payload)`: Ingests telemetry, incoming tokens, or state shifts from the Orchestrator.
        

B. The Cockpit Topology

The terminal grid is partitioned into four rigid zones.

- **Zone 1: The Primary Viewport (Center/Left)**
    
    - _Function:_ Renders the conversation stream.
        
    - _Styling Rules:_ Final JSON-extracted `message_to_user` text is rendered normally. The `<think>` monologue piped from the Reasoning Router (Block S01.1) is explicitly rendered in dim grey/purple to visually separate latent reasoning from actual output. System errors (from the Fuckup Loop) render in red.
        
- **Zone 2: The Pulse & The Avatar (Top Right)**
    
    - _Function:_ The state indicator and personality matrix, mapped directly to the `AgentState` enum (Block S04) with zero LLM generation overhead.
        
    - _Visuals:_
        
        - `Idle`: `[ - _ - ]` (Dim blue - Sleeping/Compacting)
            
        - `Chat`: `[ ^ _ ^ ]` (Blinking green - Active)
            
        - `Reflect`: `[ ~ _ ~ ]` (Solid amber - Processing Memory)
            
        - `Recover`: `[ O _ O ]` (Flashing red - Fuckup Loop / Panic)
            
- **Zone 3: Telemetry (Right Sidebar)**
    
    - _Function:_ Live hardware and state tracking.
        
    - _Context Diet:_ A progress bar tracking `current_tokens` vs. `AppConfig.num_ctx` (e.g., `[|||||| ] 65%`). Turns orange approaching the condensation threshold.
        
    - _Ephemeral Load:_ Displays the current entry count of the `moka` cache (e.g., `Staged Facts: 3/10000`).
        
    - _Retrieval Stats:_ Shows latency and hit count of the last `memory:query` execution.
        
- **Zone 4: Command & Notification Deck (Bottom)**
    
    - _Function:_ User input (`stdin`) and system toasts.
        
    - _OS Integration:_ When the file watcher (Block S03) detects a new `.md` or `.txt` file in the Drops folder, it fires an `mpsc` message here. The bar flashes: `[INGEST] File 'spec.md' staged in sensory cache.`
        

C. The Input Lock & Interrupt Routing

To prevent context collisions, the input handling strictly follows the `AgentState`:

- **During Chat, Reflect, or Recover:** The input bar physically locks. It ignores standard keystrokes to ensure the LLM is not interrupted mid-cycle. (Hard exit via `Ctrl+C` remains active).
    
- **During Idle:** The input bar is technically locked, but the keystroke listener remains hot. As defined in Block S04.3, pressing any key immediately drops the `tokio` future, triggering the Wake Sequence.
    
- **During WAIT_FOR_USER:** The bar unlocks for standard text entry.
    

3. Acceptance Criteria
    

- [ ] The binary launches the TUI, claims the terminal buffer via `crossterm`, and renders the layout partitions without flickering.
- [ ] Incoming token streams update the Primary Viewport asynchronously; the UI remains responsive to terminal resizing during generation.
- [ ] A state change in the Orchestrator instantly updates Zone 2 to the correct ASCII Avatar and color status via `mpsc` messaging.
- [ ] Dropping a `.txt` file into the local OS Drops directory successfully triggers a visual toast in Zone 4 without halting the primary chat loop.
- [ ] Pressing a key while Zone 2 displays the `Idle` avatar successfully triggers the Wake Sequence and unlocks the input bar for the next turn.