# Handover: doc-summarize long-document workflow (2026-06-25)

## What was done

Changes to support multi-step paginated `doc:read` for full-document summarization (like reading all 1607 chunks of "The Order of Things"). All changes are in unstaged working tree.

### Code changes (all in `src/`)

1. **Tool round cap lifted during Reflect** (`orchestrator/core/step.rs`)
   - The `max_tool_rounds` check now skips when `self.state == AgentState::Reflect`
   - Previously capped at 5, which killed the workflow after 4 tool calls

2. **Reflect continuation guidance** (`orchestrator/llm_support/post_tool_guidance.rs`)
   - New `POST_TOOL_REFLECT_CONTINUATION_GUIDANCE` injected instead of `POST_TOOL_USER_REPLY_GUIDANCE` when state is Reflect
   - Tells the model to keep going with Reflect + tool_calls, not stop early with Idle

3. **vault:write allowed in Reflect** (`tools/gatekeeper.rs`)
   - Added to the Reflect allowlist so the doc-summarize skill can write running notes
   - `state_allows_tool` made `pub` for use in step.rs and llama_gbnf_subset.rs

4. **vault:write auto-offered with doc:read** (`orchestrator/core/step.rs`, `orchestrator/core/llama_gbnf_subset.rs`)
   - When doc:read is in the offered tool set, vault:write is auto-added (needed for note-taking)
   - Duplicated in both the step.rs slim assembly and the GBNF subset builder

5. **doc:read gets Full context view hint** (`tools/doc/read.rs`)
   - Changed from `Default` (truncated to 2500 chars) to `Full` so the model can see the content it just read

6. **Context pruning utility** (`orchestrator/context/prune.rs`) — NEW FILE
   - `prune_stale_tool_results(&mut chat_stack, tool_name, keep_last_n)` replaces old tool results with compact markers
   - Wired in `tool_dispatch.rs`: after successful doc:read in Reflect, prunes all but the latest result

7. **Config changes** (`config.rs`, `memory/document_store.rs`)
   - `max_chunks_per_doc` raised from 500 to 4000
   - New `cleanup_source_after_ingest` option (post-ingest file deletion)

## Test run results (00:39–00:41 UTC)

### What worked
- `max_tool_rounds` was raised to 40 in the running config — no cap hit
- Pruning is firing correctly: `pruned=1 kept=1` on every round after the 3rd
- Context size plateaued at ~21-25K chars (after view), prompt tokens at ~5.3-6.9K — no timeout
- Model ran 13 successful doc:read rounds (rounds 1-13) without crashing
- Generation times stable at 4-5 seconds per round
- The model produced a reasonable summary in the end

### Three open problems

**1. Model never called vault:write (critical)**
Zero vault:write calls in the entire run. The model keeps saying in its thoughts "I will also start keeping notes" but never does it. It just chains doc:read after doc:read, then eventually gives up and dumps a summary from whatever it still has in context.

Root causes to investigate:
- The `POST_TOOL_REFLECT_CONTINUATION_GUIDANCE` doesn't specifically mention vault:write or note-taking
- The model may be prioritizing "read more" over "write notes now"
- Gemma-4-12b might not be following the doc-summarize skill guidance closely enough (skill says "After each page, append running section notes via vault:write")
- The GBNF grammar allows multiple tool_calls per response but the model only emits one at a time

Possible fixes:
- Make the continuation guidance explicitly say "write notes to vault before requesting the next page"
- Or inject a hard nudge after every N doc:read rounds: "You have read N pages without writing notes. Use vault:write now."
- Or have the orchestrator auto-inject a vault:write after each doc:read with a summarization prompt (heavy-handed)

**2. Model stopped at chunk 140 of 1607 (only ~8.7% of the document)**
After 12 doc:read calls (chunks 0-154), the model went Idle and produced a summary. It covered the foreword, TOC, and first couple of chapters. The `POST_TOOL_REFLECT_CONTINUATION_GUIDANCE` says "Do not stop early with a progress report" but the model still stopped.

Possible causes:
- Context pressure — even with pruning, the overhead from assistant responses and guidance messages grows linearly (~1K per round for thought+guidance)
- Model fatigue — Gemma-4-12b-it may lose the thread of the instruction after many rounds
- The growing stack of pruned markers + assistant compact strings may be confusing the model into thinking it's done
- 12 rounds of identical "Reflect + doc:read" may trigger some implicit repetition avoidance

**3. Chunk ordering has gaps**
The sequence was: 0, 15, 30, **38**, 53, **64**, 78, 93, **106**, 120, **130**, 140

Expected (at 15/page): 0, 15, 30, 45, 60, 75, 90, 105, 120, 135, 150...

The model is mostly doing +15 but sometimes miscounts (38 instead of 45, 64 instead of 75, 106 instead of 105, 130 instead of 135). It's losing track of the exact offset. This means chunks 45-52, 75-77, etc. are skipped entirely.

Possible fixes:
- Have the doc:read tool result explicitly say "next page: use start=X" (it already says "use start=X to continue" but the model sometimes ignores the exact number)
- Or have the orchestrator auto-correct the `start` parameter if it detects a gap
- Or increase page size to 40 (as the skill says) so fewer pages = fewer opportunities to miscalculate

## Files changed (git diff --stat)

```
 src/config.rs                                      |  9 +++++++-
 src/memory/document_store.rs                       | 26 ++++++++++++++++++++
 src/orchestrator/context/mod.rs                    |  2 ++
 src/orchestrator/context/prune.rs                  | 153 +++++++++++++  (NEW)
 src/orchestrator/core/llama_gbnf_subset.rs         |  7 ++++++
 src/orchestrator/core/step.rs                      | 20 ++++++++++++--
 src/orchestrator/core/tool_dispatch.rs             | 25 ++++++++++++++----
 src/orchestrator/llm_support/post_tool_guidance.rs |  7 ++++++
 src/tools/doc/read.rs                              |  5 +++++
 src/tools/gatekeeper.rs                            | 23 ++++++++---------
```

## Test status

All 32 tests pass (`cargo test -- prune_stale context_view gatekeeper llama_gbnf`).

## Priority for next session

1. Fix vault:write not being called — this is the blocker for actually summarizing large documents correctly
2. Fix chunk ordering gaps — either in the skill guidance, the tool output, or with orchestrator correction
3. Consider increasing page size from 15 to 40 as the skill recommends
