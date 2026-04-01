
#fcp

1. Objective
    
    Define the exact native Rust functions that act as the system's sensory input and executive output. The LLM cannot execute code; it can only request these predefined operations via the Gatekeeper (Block S05). Every tool must adhere strictly to the "Context Diet," returning only high-signal, compressed data to prevent blowing out the VRAM limits. Crucially, it leverages Qdrant as a "semantic rasterizer" to handle massive files without fatal pagination loops.
    
2. Architecture & Design Rules
    

A. The Execution Contract

Every tool in this manifest follows the strict internal Rust signature established in S05.1: `async fn execute(&self, args: &str) -> crate::error::Result<String>`

- The returned `String` is the exact text injected into the LLM's `[CHAT_STACK]` for the next turn.
    
- If a tool fails, the `Err(FcpError)` is caught by the Orchestrator (Block S04) and fed back to the LLM as a `FUCKUP DETECTED` string to trigger the recovery loop.
    

B. Analytic Tools (Memory & State)

Tools designed to query the internal databases and system metrics.

- **`memory:query` (The Semantic Zoom)**
    
    - _Args:_ `query` (String), `filter_tag` (Optional String), `file_path` (Optional String).
        
    - _Logic:_ Calls `OllamaClient::embed()` for the query and searches the dynamically resolved Qdrant collection (`fcp_vault_{workspace}`). If `file_path` is provided, Qdrant physically filters the vector search to return chunks _only_ from that specific file (Semantic Zoom).
        
    - _Constraint:_ Hard-cap the return string to 1500 tokens. If results exceed this, truncate and append `[RESULTS TRUNCATED]`.
        
- **`system:health`**
    
    - _Args:_ None.
        
    - _Logic:_ Pings the local Qdrant port, reads active `moka` cache size, and checks the workspace lockfile seal.
        
    - _Return:_ A dense JSON string of system vitals.
        

C. Executive Tools (Action & Persistence)

Tools designed to stage facts or write them permanently to the Vault.

- **`memory:stage`**
    
    - _Args:_ `content` (String), `tag` (String).
        
    - _Logic:_ Injects the content into the `moka` cache (Block S03.1) wrapping it with the absolute `SystemTime`.
        
    - _Return:_ `SUCCESS: Fact staged in Ephemeral cache.`
        
- **`memory:commit`**
    
    - _Args:_ `tag` (String), `target_domain` (Enum: "Semantic", "Episodic").
        
    - _Logic:_ Pulls all `moka` cache entries matching the tag. Formats them into MemCollab-compliant invariant rules. Appends them to `vaults/{workspace}/20_Semantic/{tag}.md` or `10_Episodic/{date}.md`. It then invalidates those keys in `moka`.
        
    - _Trigger:_ This physical file write automatically fires the OS `notify` watcher (Block S03), which generates Nomic embeddings and upserts to Qdrant.
        
    - _Return:_ `SUCCESS: Flushed {n} facts to Vault.`
        

D. Sensory Tools (Direct File I/O)

Tools for reading/writing exact files when semantic vector search is too broad or inaccurate.

- **`vault:read` (The Map Fallback)**
    
    - _Args:_ `relative_path` (String).
        
    - _Logic:_ Reads a specific `.md` or `.txt` file from the active `vaults/{workspace}/`.
        
    - _Constraint:_ If the physical file token count exceeds 3000 tokens, the tool refuses the full read. Instead, it natively regex-parses the file, extracts only the Markdown headers (`##`), and returns: `ERROR: File exceeds 3000 tokens. Use memory:query to search it semantically. FILE MAP: [## Header 1, ## Header 2, ...]`.
        
- **`vault:write`**
    
    - _Args:_ `relative_path` (String), `content` (String), `mode` (Enum: "overwrite", "append").
        
    - _Logic:_ Writes strings directly to the physical disk inside the workspace.
        
    - _Constraint:_ Banned from targeting `00_Core/`. Caught by the Gatekeeper path firewall prior to execution.
        
- **`vault:list`**
    
    - _Args:_ `directory` (String).
        
    - _Logic:_ Returns a flat list of file paths in a specified Vault subdirectory.
        

3. Acceptance Criteria
    

- [ ] Attempting to `vault:read` a massive log file successfully aborts the full read, parses the file natively, and returns the condensed Header Map to guide the LLM's next move.
- [ ] Providing a `file_path` to `memory:query` successfully limits the Qdrant cosine similarity search exclusively to vectors originating from that specific document.
- [ ] Executing `memory:stage` immediately reflects the new data size when `system:health` is subsequently called.
- [ ] Executing `memory:commit` successfully moves data from the `moka` cache to the physical Markdown file and clears the specific keys from RAM.
- [ ] Attempting to `vault:write` to `00_Core/Identity.md` is trapped and rejected by the Gatekeeper.
 