### 2. Block S08: Semantic Memory (The Vector Brain)

- **Goal:** Breathe life into the `memory:query` and `memory:commit` stubs.
- **Tasks:** - Integrate `qdrant-client` into the Rust binary.
  - Wire `nomic-embed-text` via Ollama API to generate vector embeddings.
  - Implement RAG (Retrieval-Augmented Generation) so ERIS can search her own Vault.
    ld the `fcp log` CLI bypass to read the black box.

### 5. clean up UI

**findings**

- show cursor pos
- show prev prompt and answer
- respect grid end - text overflows out of view // scalable?
- maybe a new section to display ERIS errors?
- /exit should exit and close program
