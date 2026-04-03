# Critical Architecture Review

This document provides a pragmatic, no-fluff review of the Eris architecture, highlighting mistakes, critical flaws, and areas for improvement.

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