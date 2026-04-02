# Critical Architecture Review

This document provides a pragmatic, no-fluff review of the Eris architecture, highlighting mistakes, critical flaws, and areas for improvement.

## 1. Deadlock Prevention vs. Actor Model Implementation

**The Rule:** *"Do not share mutable state across threads using `Arc<Mutex<T>>`. Threads must communicate strictly through `tokio::sync::mpsc` message passing (as defined in the TUI/Orchestrator architecture)."*

**The Reality:** The architecture is a hybrid, not a pure Actor model. While the TUI does strictly use `mpsc` for state broadcasts and user actions, the core memory structures (`EphemeralMemory` and `Gatekeeper`) are shared across boundaries using `Arc`. `EphemeralMemory` relies on `moka::future::Cache` which internally handles concurrency without explicitly exposing a `Mutex`, but it remains shared memory state. 
**Improvement:** If strict actor isolation is truly desired, `EphemeralMemory` should run in its own spawned task and process commands via `mpsc::Receiver<MemoryCommand>`. Currently, passing `Arc<EphemeralMemory>` to multiple tools violates the spirit of the rule, even if it avoids a literal `Mutex`.

## 2. Unnecessary Latency in Semantic Routing

**The Flaw:** On *every single turn* (unless bypassed by the short-input guard), the `ToolRouter` makes an HTTP call to Ollama to generate an embedding for the user's input to calculate cosine similarity.
**The Impact:** This introduces a minimum latency floor of 50-200ms (depending on the embedding model) before the actual generation even starts, just to decide which tool schemas to inject.
**Improvement:** 
- Cache common intent embeddings locally.
- Use a lightweight, in-process Rust embedding crate (e.g., `candle` or `ort` with a tiny ONNX model) rather than relying on the heavier `ollama-rs` HTTP interface for simple semantic routing.

## 3. O(N) Boot-Time Vault Ingestion

**The Flaw:** `ingest_vault` synchronously reads, parses, and sends HTTP embedding requests for *every* non-empty markdown file in specific subdirectories upon startup.
**The Impact:** As the vault grows to thousands of files, Eris will experience catastrophic boot delays.
**Improvement:** 
- Decouple ingestion into an asynchronous background task that runs *after* the TUI has loaded.
- Implement a local hash-state cache (e.g., tracking file modification times or SHA hashes) to only embed and upsert files that have changed since the last boot.

## 4. Brutal Context Condensation

**The Flaw:** When the token count exceeds the `condensation_threshold`, `execute_condensation` requests a JSON summary of the entire conversation and *completely replaces* the `chat_stack` with that single summary message.
**The Impact:** The agent immediately suffers severe amnesia regarding specific conversational nuances, formatting instructions, or recent precise data.
**Improvement:** Implement a sliding window or a rolling summary. Retain the most recent `N` messages untouched while only summarizing the older history.

## 5. Vulnerability to Vector DB Outages

**The Flaw:** The `SemanticBrain` attempts to connect to Qdrant with retries. If it fails, it returns `None`. However, tools like `web:fetch` and `web:artifact_query` are heavily integrated with Qdrant for semantic chunking of web pages.
**The Impact:** If Qdrant goes down, the web scraping capabilities degrade silently or fail entirely. 
**Improvement:** Implement an ephemeral, purely in-memory vector index fallback for web chunking when Qdrant is unavailable, ensuring that core tools remain functional even if the semantic database is down.

## 6. Hardcoded Vault Paths

**The Flaw:** Directories like `"10_Episodic"`, `"20_Semantic"`, `"30_Persons"`, and `"40_User"` are hardcoded in `semantic.rs`.
**The Impact:** Forces the user into a specific folder taxonomy, breaking compatibility with existing Obsidian structures.
**Improvement:** Expose these paths in `AppConfig` or `.fcp/config.toml` so users can map semantic categories to their own directory layouts.

## 7. Brittle LLM Output Extraction

**The Flaw:** `extract_json_slice` attempts to find the first `{` and last `}` to parse the LLM's response into a `LoopDirective`.
**The Impact:** If the model includes a JSON code block inside a `<think>` tag (e.g., reasoning about what JSON it *should* generate), the greedy `{...}` slice will capture the reasoning tag's braces, resulting in a JSON parsing failure.
**Improvement:** Strip `<think>` tags *before* attempting JSON extraction. (The `ReasoningRouter` exists in the codebase but its integration point needs to guarantee that thought blocks are completely excised before `process_llm_response` fires).