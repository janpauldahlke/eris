
#fcp

1. Objective
    
    Establish the permanent, authoritative long-term memory system. This block bridges the local Obsidian Markdown vault with a local Qdrant vector database. It enforces physical immutability for core directives while allowing the agent to recall specific knowledge using strict metadata tag filtering. Crucially, it mandates parallel VRAM execution to guarantee zero-latency semantic retrieval without PCIe hot-swapping bottlenecks.
    
2. Architecture & Design Rules
    

A. The Physical Topology & Template Cloning
The system expects a parent `vaults/` directory containing isolated workspaces. 
* The Active Path: `vaults/{workspace}/`.
* The Clone Init: If `fcp` boots and `vaults/{workspace}/` does not exist, the Orchestrator MUST execute a recursive file-system copy of `vaults/_template/` to `vaults/{workspace}/`.
* The Internal Structure (per workspace):
    * `00_Core/` (THE IMMUTABLE ZONE): Contains `Identity.md`. Hardcoded Gatekeeper read-only rule: `vault:write` cannot target this path.
    * `10_Episodic/`: Time-series memory.
    * `20_Semantic/`: Procedural and factual knowledge.
    * `90_Drops/`: Asynchronous file ingestion zone.

B. The Hardware Seal (Cross-Contamination Lock)
To prevent MemCollab poisoning across models, a workspace is physically bound to the model that initialized it.
* The Seal: Upon successful initialization of a new workspace (or template clone), the Orchestrator writes a hidden lockfile: `vaults/{workspace}/.fcp_seal.json`.
* The Payload: `{"model": "qwen3.5:9b"}` (Extracts the exact string from `AppConfig.model_name`).
* The Boot Check: On every subsequent boot, the Orchestrator reads the seal. If the current `AppConfig.model_name` does not exactly match the seal's model string, the boot is aborted with `Err(FcpError::Config("Model mismatch. Workspace is sealed to {sealed_model}")).`


C. The Embedding Pipeline & VRAM Physics (`src/memory/embedding.rs`)

We strictly use `nomic-embed-text-v1.5` hosted via the local Ollama daemon. Because hot-swapping a 9B/32B model out of VRAM to load a 400MB embedding model takes 2-4 seconds, both models **must** reside in VRAM simultaneously.

- **The Daemon Environment:** The host system running Ollama must be configured with `OLLAMA_MAX_LOADED_MODELS=2` and `OLLAMA_NUM_PARALLEL=2` to permit side-by-side execution.
    
- **The Keep-Alive Lock:** When `OllamaClient::embed()` or `OllamaClient::generate()` is called, the HTTP payload **MUST** include `"keep_alive": "1h"`. This prevents Ollama's aggressive garbage collector from evicting Nomic or Qwen between tool calls.
    
- **The Context Win:** Because Nomic supports 8192 tokens, chunking is delimited strictly by Markdown Header 2 (`##` ) or double newlines (`\n\n`), targeting ~300-500 words per chunk. Prepended format: `Path: 20_Semantic/Rust.md | Header: Memory Safety | Content: {chunk}`.



D. The Qdrant Link & Workspace Isolation (`src/memory/qdrant.rs`)

Qdrant runs as a sidecar daemon. We use it specifically for its JSON payload filtering.

- **Tooling:** `qdrant-client` via gRPC or HTTP.
    
- **Isolation:** The target collection is resolved dynamically from `AppConfig` as `fcp_vault_{workspace}`. If you boot with `fcp --workspace qwen_9b`, reads/writes strictly hit the `fcp_vault_qwen_9b` collection.
    
- **The Abstraction Trait:** To prevent CI pipelines from requiring a live Qdrant container, the database is abstracted.
    
    Rust
    
    ```
    use async_trait::async_trait;
    use crate::error::Result;
    
    #[async_trait]
    pub trait VectorStore: Send + Sync {
        async fn search(&self, query_vector: Vec<f32>, filter_tag: Option<&str>) -> Result<Vec<VaultPayload>>;
        async fn upsert(&self, id: String, vector: Vec<f32>, payload: VaultPayload) -> Result<()>;
    }
    ```
    

E. The Payload Schema

Every vector inserted into Qdrant must carry this exact metadata JSON payload to allow the Gatekeeper to filter effectively.

JSON

```
{
  "file_path": "10_Episodic/2026-03-27_Log.md",
  "domain": "episodic", 
  "tags": ["#rust", "#architecture"],
  "chunk_index": 3,
  "inserted_at": 1711533600,
  "content": "Raw text returned to the LLM."
}
```

E. The Vault Watcher & Drop Zone Physics

The vector database must accurately reflect the physical state of the Obsidian vault in real-time, fortified by defensive file-parsing rules.
* **Tooling:** The `notify` crate hooks into OS-level file system events.
* **The Whitelist Firewall:** The watcher strictly filters incoming events. It ONLY processes files with the following extensions: `["md", "txt", "rs", "json", "toml", "csv"]`. All other extensions (e.g., binaries, PDFs, images) are silently ignored to prevent UTF-8 parsing panics.
* **Flat Drop Topology:** The watcher is configured to ignore directory creation events within `90_Drops/`. Files must be placed at the root of the drop zone.
* **Event Loop:** When `EventKind::Modify` or `Create` fires on a whitelisted file:
    1. Debounce the event (wait 500ms to ensure the file write is complete).
    2. Read file as UTF-8, chunk by headers/newlines, extract `#tags` using regex.
    3. Call `OllamaClient::embed()` (with `keep_alive` set).
    4. Upsert to Qdrant, overwriting old vectors sharing the same `file_path` and `chunk_index`.
* **The Sensory Alert:** If the file was specifically created in `90_Drops/`, the watcher stages a notification in the `moka` cache (Block S03.1) using the key `sensory:{unix_timestamp}.file_drop`. The physical file remains in `90_Drops/` until manually moved by the user or the agent.

3. Acceptance Criteria
    

- [ ] All HTTP requests to Ollama (both `/api/chat` and `/api/embeddings`) successfully inject the `"keep_alive": "1h"` parameter to lock the models in VRAM.
- [ ] The `vault:write` tool successfully intercepts any attempt to modify files within `00_Core/`, returning a structured `ToolFault`.
- [ ] Modifying a Markdown file in the vault automatically triggers the `notify` watcher, requests embeddings, and upserts the payload to the dynamically named `fcp_vault_{workspace}` collection.
- [ ] A `memory:query` request with a specific `filter_tag` successfully ignores highly similar vectors that lack the matching tag.
- [ ] Unit tests for memory retrieval successfully utilize a `MockVectorStore` trait implementation, passing without requiring a live Qdrant connection.
- [ ] Booting a non-existent workspace successfully clones the `_template` directory and generates a `.fcp_seal.json` lockfile containing the active model name.
- [ ] Attempting to boot an existing workspace with a `.env` model string that contradicts the `.fcp_seal.json` successfully aborts the process with `FcpError::Config`.
- [ ] Dropping a binary file (e.g., `image.png`) or an unsupported document (`spec.pdf`) into the vault successfully bypasses the ingestion loop without throwing a Rust panic or crashing the thread.
- [ ] Dropping a nested directory into `90_Drops/` is successfully ignored by the watcher.
