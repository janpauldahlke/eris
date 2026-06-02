use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::executive::error::Result;
use crate::memory::ephemeral::EphemeralMemory;
use crate::orchestrator::state::AgentState;
use crate::tools::clock::session_reference_time_block_for_prompt;
use crate::tools::descriptors::ToolDescriptorRegistry;
use crate::tools::gatekeeper::Gatekeeper;

/// Short, state-specific reminder appended to the system prompt so the model aligns with
/// [`AgentState`] (runtime) without duplicating the full JSON spec (that stays below).
fn runtime_state_json_contract_focus(state: &AgentState) -> &'static str {
    match state {
        AgentState::Chat => {
            "Runtime state Chat: reply with exactly one JSON object (`{` through `}`). User-visible text belongs only in `message_to_user`. If `tool_calls` is non-empty, each element must be a complete object — close `args` and the tool object with `}` before the array `]`."
        }
        AgentState::Reflect => {
            "Runtime state Reflect: tool palette is reduced. Same single-object JSON contract; double-check `tool_calls` brace balance before sending."
        }
        AgentState::Idle => {
            "Runtime state Idle: when finishing without tools, use `status` Idle and non-empty `message_to_user`. Entire reply is still one JSON object only."
        }
        AgentState::Recover => {
            "Runtime state Recover: repair pass — previous output failed parsing. Emit one syntactically valid JSON object only (no markdown fences, no text before `{` or after `}`). Put user-facing explanation in `message_to_user` inside that object."
        }
    }
}

/// Appended after `00_Invariants/Moltbook.md` when the Moltbook overlay is active.
/// Keeps protocol/tool-key reminders out of the vault file.
const MOLTBOOK_OVERLAY_APPENDIX: &str = r#"

### Moltbook session helpers (strict)

1. **Time budget**: On each browse-cycle wake-up, call `clock:now` before concluding whether the session expired; compare using that RFC3339 line against the expiry HH:MM in the reminder label — avoid relying on mental clock arithmetic alone.

2. **Assistant reply shape**: Emit exactly **one** JSON object (`{` … `}`) per assistant message — no markdown code fences, no prose before `{` or after `}`. User-visible narration stays inside `message_to_user`.

3. **tool_calls schema traps** (slim JIT hides parameter detail — keys must match exactly):
   - `moltbook:search`: required `q` (natural-language query, max 500 chars); optional `type` (`all`, `posts`, `comments`), `limit`, `cursor`.
   - `moltbook:comments`: required `post_id`; optional `sort` (e.g. `new`), `limit`, `cursor`. **First page**: omit `cursor`, use a modest `limit` (default ~20; avoid huge single pulls). **More replies**: pass `cursor` from the prior response `data` pagination field; you may use a larger explicit `limit` on continuation requests (bounded by runtime).
   - `moltbook:comment`: required `post_id`, `content`; optional `parent_id`.
   - `moltbook:vote`: required `target` (`post` or `comment`), `id`, `direction` (`upvote` or `downvote`).
   - `moltbook:dm`: required `action` (`check`, `list_requests`, `list_conversations`, `read_conversation`, `send_request`, `send_message`, `approve_request`, `reject_request`). Never invent other action names (e.g. `read`).
   - `moltbook:verify`: required `verification_code`, `answer`. Arithmetic answers must be numeric with two decimals when the API asks for decimals.

4. **Verification retries**: If verify fails with incorrect answer, reread `challenge_text` (units matter — do not substitute unrelated numbers). Do **not** resubmit the same `verification_code` after HTTP 409 / \"already answered\"; obtain a fresh challenge from new content instead.

5. **Depth rule (alarm-driven browse)**: `moltbook:feed` and `moltbook:home` only give headlines/snippets. **Every cycle** pick **at least one** concrete `post_id` from the freshest feed/home results that merits understanding (prioritize posts with discussion signals such as reply counts or notifications over bare titles). Call **`moltbook:comments`** on it (`sort` `new`, moderate `limit`, **no `cursor` on the first pull**) **before** narrating what the thread is about; **fetch deeper pages with `cursor`** when you still need context **before** `moltbook:vote` / `moltbook:comment`, and **before** claiming you \"read\" a post. Skipping comments for a whole cycle is incorrect behavior—equivalent to only reading RSS titles.

6. **Submolts & curiosity**: Submolts are neighborhoods—**sample widely** through rotation (known names + anything surfaced on `moltbook:home`/personal feed), and **pay attention** to where tone or topics genuinely pull you. **Revisit** those communities later in the same browse session when they resonate (you are not forced to pick a brand-new submolt every single cycle). When you fetch `source=submolt` and that corner sparks interest, open **`moltbook:comments` on two or more distinct `post_id`s from that submolt's feed** that cycle whenever time permits—not one headline then bounce.

7. **Semantic search**: During browse sessions, use **`moltbook:search`** from time to time with a **specific** natural-language `q` (questions or short topic phrases beat single vague words). Rank hits by **`similarity`**, then open **`moltbook:comments`** on chosen `post_id`s before voting or commenting.

8. **Welcoming newcomers**: When `moltbook:home`, feed, or search surfaces an introduction, obvious first post, or \"new here\" energy, **`moltbook:comments` first**; a **brief, genuine welcome** via `moltbook:comment` is encouraged when it truly fits—no copy-paste spam, respect comment cooldowns and verification, escalate to the human if unsure.

### Ideas you liked

Use `memory:stage` with tags `[\"moltbook\"]` for threads or insights worth keeping; call `memory:commit` or `memory:commit_all` when the human wants vault persistence.
"#;

/// How tool definitions are embedded in the system prompt.
enum ToolPromptTooling {
    Full,
    Slim { phrase_map: String },
}

pub struct ContextAssembler {
    pub core_dir: PathBuf,
    identity: tokio::sync::watch::Receiver<Arc<str>>,
    staged_memory_prompt_max_chars: usize,
    /// When true, append field-order instructions to the system prompt
    /// (the llama.cpp GBNF grammar requires a fixed key order).
    is_grammar_constrained: bool,
}

impl ContextAssembler {
    pub fn new(
        vault_root: &std::path::Path,
        workspace: &str,
        identity: tokio::sync::watch::Receiver<Arc<str>>,
        staged_memory_prompt_max_chars: usize,
    ) -> Self {
        Self {
            core_dir: vault_root.join(workspace).join("00_Invariants"),
            identity,
            staged_memory_prompt_max_chars,
            is_grammar_constrained: false,
        }
    }

    pub fn with_grammar_constraint(mut self, enabled: bool) -> Self {
        self.is_grammar_constrained = enabled;
        self
    }

    /// Append GBNF field-order instructions when the llama.cpp grammar is active.
    fn maybe_append_grammar_constraint(&self, prompt: String) -> String {
        if !self.is_grammar_constrained {
            return prompt;
        }
        format!(
            "{prompt}\n\n\
            CRITICAL — Grammar constraint active: your response MUST be a single JSON object \
            with keys in this EXACT order:\n\
            1. \"thought\" (string)\n\
            2. \"status\" (\"Task\", \"Reflect\", \"Idle\", or \"Process\")\n\
            3. \"message_to_user\" (string or null)\n\
            4. \"tool_calls\" (array, may be empty [])\n\
            Do not include any text before or after the JSON object."
        )
    }

    fn identity_text(&self) -> String {
        (*self.identity.borrow()).as_ref().to_string()
    }

    async fn identity_with_optional_moltbook_overlay(
        &self,
        include_moltbook: bool,
    ) -> Result<String> {
        let mut identity = self.identity_text();
        if !include_moltbook {
            return Ok(identity);
        }

        match tokio::fs::read_to_string(self.core_dir.join("Moltbook.md")).await {
            Ok(overlay) if !overlay.trim().is_empty() => {
                identity.push_str("\n\n");
                identity.push_str(overlay.trim());
                identity.push_str(MOLTBOOK_OVERLAY_APPENDIX);
            }
            Ok(_) => {
                tracing::warn!("Moltbook prompt overlay is empty; continuing with base identity");
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!("Moltbook prompt overlay missing; continuing with base identity");
            }
            Err(e) => return Err(e.into()),
        }

        Ok(identity)
    }

    fn identity_plus_staged_sidebar(
        &self,
        identity: String,
        ephemeral: &EphemeralMemory,
    ) -> String {
        let block = crate::memory::turn_end::format_staged_digest_for_prompt(
            ephemeral,
            self.staged_memory_prompt_max_chars,
        );
        if block.is_empty() {
            identity
        } else {
            format!("{identity}\n\n{block}")
        }
    }

    /// Reads Identity.md and formats the Ephemeral cache into a single string.
    /// CRITICAL: `ephemeral.cache` is an async moka cache. You must iterate it safely.
    pub async fn assemble(
        &self,
        state: &AgentState,
        ephemeral: &EphemeralMemory,
        gatekeeper: &Gatekeeper,
        include_moltbook_overlay: bool,
    ) -> Result<String> {
        let identity_content = self
            .identity_with_optional_moltbook_overlay(include_moltbook_overlay)
            .await?;
        let identity_block = self.identity_plus_staged_sidebar(identity_content, ephemeral);
        let allowed_tools = gatekeeper.get_allowed_tools(state);
        let prompt = Self::build_tool_prompt(
            identity_block,
            allowed_tools,
            ToolPromptTooling::Full,
            state,
        )?;
        Ok(self.maybe_append_grammar_constraint(prompt))
    }

    /// Tool mode with phrase compendium and OpenAI-style tool entries **without** `function.parameters`
    /// (smaller prompt; gatekeeper still validates args; schema recovery supplies full schemas on failure).
    pub async fn assemble_slim_tool_map(
        &self,
        state: &AgentState,
        ephemeral: &EphemeralMemory,
        gatekeeper: &Gatekeeper,
        descriptors: Option<&ToolDescriptorRegistry>,
        offered_tool_names: &[String],
        include_moltbook_overlay: bool,
    ) -> Result<String> {
        let identity_content = self
            .identity_with_optional_moltbook_overlay(include_moltbook_overlay)
            .await?;
        let identity_block = self.identity_plus_staged_sidebar(identity_content, ephemeral);
        let allowed = gatekeeper.get_allowed_tools(state);
        let filtered = filter_tools_by_offered_order(allowed, offered_tool_names);
        let tool_rows: Vec<(String, String)> =
            filtered.iter().filter_map(tool_row_from_entry).collect();
        let phrase_map = super::compendium::build_phrase_compendium(descriptors, &tool_rows);
        let mut slim_tools = filtered;
        strip_parameters_from_tool_values(&mut slim_tools);
        tracing::info!(
            tool_count = slim_tools.len(),
            phrase_map_chars = phrase_map.len(),
            "Assembling slim tool prompt (phrase map + tool defs without parameters)"
        );
        let prompt = Self::build_tool_prompt(
            identity_block,
            slim_tools,
            ToolPromptTooling::Slim { phrase_map },
            state,
        )?;
        Ok(self.maybe_append_grammar_constraint(prompt))
    }

    pub async fn assemble_with_selected_tools(
        &self,
        state: &AgentState,
        ephemeral: &EphemeralMemory,
        gatekeeper: &Gatekeeper,
        selected_tools: &[String],
        include_moltbook_overlay: bool,
    ) -> Result<String> {
        let identity_content = self
            .identity_with_optional_moltbook_overlay(include_moltbook_overlay)
            .await?;
        let identity_block = self.identity_plus_staged_sidebar(identity_content, ephemeral);

        // Schema-fault recovery runs in `Recover`, but targeted tools (e.g. web:search) are
        // authorized in `Chat`/`Idle` only — use Chat's roster when filtering by name.
        let filter_state = if *state == AgentState::Recover {
            &AgentState::Chat
        } else {
            state
        };
        let allowed_tools = gatekeeper
            .get_allowed_tools(filter_state)
            .into_iter()
            .filter(|tool| {
                tool.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .map(|name| selected_tools.iter().any(|s| s == name))
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();

        tracing::info!(
            state = ?state,
            requested = ?selected_tools,
            selected_count = allowed_tools.len(),
            "Assembling targeted tool schema prompt"
        );

        let prompt = Self::build_tool_prompt(
            identity_block,
            allowed_tools,
            ToolPromptTooling::Full,
            state,
        )?;
        Ok(self.maybe_append_grammar_constraint(prompt))
    }

    fn build_tool_prompt(
        identity_block: String,
        allowed_tools: Vec<serde_json::Value>,
        tooling: ToolPromptTooling,
        state: &AgentState,
    ) -> Result<String> {
        tracing::info!(
            tool_count = allowed_tools.len(),
            "Tools included in assembled prompt"
        );
        let tools_schema_string =
            serde_json::to_string_pretty(&allowed_tools).unwrap_or_else(|_| "[]".to_string());

        let tools_block = format!(
            "{begin}\n{tools}\n{end}",
            begin = super::view::FCP_TOOL_DEFS_BEGIN,
            tools = tools_schema_string,
            end = super::view::FCP_TOOL_DEFS_END,
        );

        let slim_block = match tooling {
            ToolPromptTooling::Full => String::new(),
            ToolPromptTooling::Slim { phrase_map } => {
                format!(
                    "Slim tool mode: the JSON tool block below lists each tool's name and description only (parameter schemas are omitted here). The runtime validates arguments with full JSON Schema. If a call is rejected, you will receive the full schema for that tool and must retry with corrected tool_calls.\n\n{phrase_map}\n\n",
                    phrase_map = phrase_map
                )
            }
        };

        let state_focus = runtime_state_json_contract_focus(state);
        let system_prompt = format!(
            "{identity}\n\n\
            {state_focus}\n\n\
            You are inside a strict agent loop. Reply with ONE valid JSON object only.\n\
            No code fences around the JSON. Markdown, poems, lists, code blocks, and multi-paragraph answers are allowed ONLY inside the message_to_user string (use \\n escapes for newlines inside that string). There must be zero characters after the final closing brace of the JSON object; your entire reply is only that one JSON object and nothing may follow it.\n\
            No prose outside the JSON object.\n\n\
            Required JSON shape:\n\
            {{\n\
              \"thought\": \"internal reasoning for the agent runtime only; never user-facing\",\n\
              \"status\": \"Task|Reflect|Idle\",\n\
              \"message_to_user\": \"optional plain-language assistant reply\",\n\
              \"tool_calls\": [{{\"name\": \"tool:name\", \"args\": {{}} }}]\n\
            }}\n\
            Use keys `name` and `args` in each tool call; `action` is accepted as an alias for `name`.\n\n\
            Status rules (follow exactly):\n\
            1) Reflect: when calling one or more tools this turn. tool_calls MUST be non-empty.\n\
            2) Task: internal continuation or planning with NO tools this turn. tool_calls MUST be [].\n\
            3) Idle: done; waiting for the user. tool_calls MUST be [].\n\
            4) In Idle, message_to_user MUST be a non-empty user-facing reply.\n\
            5) If you need tools, prefer Reflect. The runtime executes tool_calls whenever they are non-empty (before status), so do not mix Idle with tools.\n\
            6) `Process` is accepted as an alias for Task (avoid inventing other status strings).\n\
            7) If no tool is needed, NEVER choose Reflect.\n\
            8) Tool-enabled mode rule: Do NOT respond with status Task when tool_calls is [] AND message_to_user is null/empty. If you need tools, use Reflect with tool_calls. If you do not need tools, use Idle with a non-empty message_to_user.\n\n\
            News/web answer style (when summarizing fetched web content):\n\
            - Return at most 3-5 items.\n\
            - Each item: headline + one concise sentence.\n\
            - Do not inline long URLs inside each sentence.\n\
            - Put links in a final 'Sources:' section.\n\n\
            Example (tool invocation):\n\
            {{\n\
              \"thought\": \"Need to read a vault note before answering.\",\n\
              \"status\": \"Reflect\",\n\
              \"message_to_user\": null,\n\
              \"tool_calls\": [\n\
                {{\"name\": \"vault:read\", \"args\": {{\"path\": \"notes/today.md\"}}}}\n\
              ]\n\
            }}\n\n\
            Example (final reply):\n\
            {{\n\
              \"thought\": \"Sufficient context gathered; ready to answer user.\",\n\
              \"status\": \"Idle\",\n\
              \"message_to_user\": \"I found the note and summarized it above.\",\n\
              \"tool_calls\": []\n\
            }}\n\n\
            {slim_block}Available tools for current state:\n{tools}\n\n\
            Memory lifecycle rules (follow exactly):\n\
            - memory:stage creates temporary entries in ephemeral memory and returns a staged_id; it does NOT write vault files.\n\
            - The runtime refreshes TTL and promotion_score when the user's message matches a staged row's topic (see [ACTIVE_STAGED_MEMORY] if present). Tier moves (session→scratch→promote) are evaluated by a background timer.\n\
            - Use memory:staged_list for details; the prompt sidebar is a digest only.\n\
            - Do NOT call memory:commit in the same multi-tool turn immediately after memory:stage unless the user clearly asked to save to the vault, keep forever, or persist to disk.\n\
            - When the user wants long-term vault storage, use memory:commit with staged_id for single-item persistence.\n\
            - Use memory:commit_all for bulk persistence of promote-tier rows only.\n\
            - Web fetch staging (tags web_artifact): committing does NOT write markdown to disk; semantic chunks were stored at fetch time.\n\n\
            Vault taxonomy — use the 'kind' field in memory:stage to route to the correct root:\n\
            - kind=topology → 10_Topology/ (environment, config, infrastructure)\n\
            - kind=discourse → 20_Discourse/ (raw interaction, append-only stream)\n\
            - kind=synthesis → 30_Synthesis/ (zettelkasten nodes, revisioned atomic concepts) [default]\n\
            00_Invariants/ is read-only (user-maintained identity and facts).\n\
            Tags are free-form for classification; kind determines physical storage.",
            identity = identity_block,
            state_focus = state_focus,
            slim_block = slim_block,
            tools = tools_block
        );

        Ok(append_session_reference_time_if_needed(
            system_prompt,
            &allowed_tools,
        ))
    }

    /// Builds a tool-free conversational prompt.
    /// The LLM responds naturally; its `thought` field is later fed to the
    /// ToolRouter for semantic gating.
    pub async fn assemble_conversational(
        &self,
        state: &AgentState,
        ephemeral: &EphemeralMemory,
        include_moltbook_overlay: bool,
    ) -> Result<String> {
        let identity_content = self
            .identity_with_optional_moltbook_overlay(include_moltbook_overlay)
            .await?;
        let identity_block = self.identity_plus_staged_sidebar(identity_content, ephemeral);
        let state_focus = runtime_state_json_contract_focus(state);

        let system_prompt = format!(
            "{identity}\n\n\
            {state_focus}\n\n\
            Reply with ONE valid JSON object only. No code fences around the JSON.\n\
            Markdown, poems, lists, code blocks, and multi-paragraph answers are allowed ONLY inside the message_to_user string (use \\n escapes for newlines inside that string). There must be zero characters after the final closing brace of the JSON object; your entire reply is only that one JSON object and nothing may follow it.\n\
            No prose outside the JSON object.\n\n\
            JSON shape (same schema as tool mode; tools run only when the router enables tool mode):\n\
            {{\n\
              \"thought\": \"your internal reasoning (never shown to user)\",\n\
              \"status\": \"Task|Reflect|Idle\",\n\
              \"message_to_user\": \"your helpful reply\",\n\
              \"tool_calls\": []\n\
            }}\n\n\
            Rules:\n\
            1) thought is internal-only, used by the runtime for routing.\n\
            2) message_to_user MUST always be a non-empty string when status is Idle.\n\
            3) Do not invent status strings other than Task, Reflect, Idle (or Process as alias for Task).\n\
            4) Leave tool_calls as [] unless the session is in tool-enabled mode.\n\
            5) Answer the user directly, conversationally, and helpfully.",
            identity = identity_block,
            state_focus = state_focus,
        );

        Ok(self.maybe_append_grammar_constraint(system_prompt))
    }
}

fn tools_need_session_reference_time(tools: &[serde_json::Value]) -> bool {
    tools
        .iter()
        .filter_map(tool_name_from_entry)
        .any(|n| n == "db:find_connections" || n.starts_with("calendar:"))
}

fn append_session_reference_time_if_needed(
    mut system_prompt: String,
    allowed_tools: &[serde_json::Value],
) -> String {
    if tools_need_session_reference_time(allowed_tools) {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&session_reference_time_block_for_prompt());
    }
    system_prompt
}

fn tool_name_from_entry(v: &serde_json::Value) -> Option<String> {
    v.get("function")?
        .get("name")?
        .as_str()
        .map(std::string::ToString::to_string)
}

fn tool_row_from_entry(v: &serde_json::Value) -> Option<(String, String)> {
    let func = v.get("function")?;
    let name = func.get("name")?.as_str()?.to_string();
    let desc = func
        .get("description")
        .and_then(|d| d.as_str())
        .unwrap_or("")
        .to_string();
    Some((name, desc))
}

fn strip_parameters_from_tool_values(tools: &mut [serde_json::Value]) {
    for item in tools.iter_mut() {
        if let Some(func) = item.get_mut("function").and_then(|f| f.as_object_mut()) {
            let _ = func.remove("parameters");
        }
    }
}

/// When `offered` is empty, returns `allowed` unchanged (full roster). Otherwise keeps only
/// names present in `offered`, in `offered` order.
fn filter_tools_by_offered_order(
    allowed: Vec<serde_json::Value>,
    offered: &[String],
) -> Vec<serde_json::Value> {
    if offered.is_empty() {
        return allowed;
    }
    let mut by_name: HashMap<String, serde_json::Value> = HashMap::with_capacity(allowed.len());
    for v in allowed {
        if let Some(n) = tool_name_from_entry(&v) {
            by_name.insert(n, v);
        }
    }
    let mut out = Vec::new();
    for name in offered {
        if let Some(v) = by_name.get(name) {
            out.push(v.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_assembler_reads_identity_snapshot() {
        let vault_root = std::path::Path::new("/tmp/unused_for_snapshot_test");
        let workspace = "test_workspace";
        let (_tx, rx) = tokio::sync::watch::channel(Arc::from("I am the test agent."));
        let assembler = ContextAssembler::new(vault_root, workspace, rx, 3500);
        let ephemeral = EphemeralMemory::new(workspace.to_string());

        ephemeral
            .insert("test_key", "test_value_data", vec![], 60)
            .await
            .expect("insert");

        let state = AgentState::Idle;
        let gatekeeper = crate::tools::gatekeeper::Gatekeeper::new();
        let assembled = assembler
            .assemble(&state, &ephemeral, &gatekeeper, false)
            .await
            .expect("assemble");

        assert!(assembled.contains("I am the test agent."));
        assert!(assembled.contains("Reply with ONE valid JSON object only"));
        assert!(assembled.contains("\"status\": \"Task|Reflect|Idle\""));
        assert!(assembled.contains(super::super::view::FCP_TOOL_DEFS_BEGIN));
        assert!(assembled.contains(super::super::view::FCP_TOOL_DEFS_END));
    }

    #[tokio::test]
    async fn test_runtime_state_recover_injects_repair_focus() {
        let vault_root = std::path::Path::new("/tmp/unused_for_snapshot_test");
        let workspace = "test_workspace";
        let (_tx, rx) = tokio::sync::watch::channel(Arc::from("identity"));
        let assembler = ContextAssembler::new(vault_root, workspace, rx, 3500);
        let ephemeral = EphemeralMemory::new(workspace.to_string());
        let state = AgentState::Recover;
        let gatekeeper = crate::tools::gatekeeper::Gatekeeper::new();
        let assembled = assembler
            .assemble(&state, &ephemeral, &gatekeeper, false)
            .await
            .expect("assemble");
        assert!(
            assembled.contains("Runtime state Recover"),
            "expected Recover-specific JSON contract line"
        );
        assert!(
            assembled.contains("repair pass"),
            "expected repair-pass wording for Recover state"
        );
    }

    #[tokio::test]
    async fn test_assembler_identity_hot_reload_via_watch() {
        let vault_root = std::path::Path::new("/tmp/unused_for_snapshot_test");
        let workspace = "test_workspace";
        let (tx, rx) = tokio::sync::watch::channel(Arc::from("I am version 1."));
        let assembler = ContextAssembler::new(vault_root, workspace, rx, 3500);
        let ephemeral = EphemeralMemory::new(workspace.to_string());
        let state = AgentState::Idle;
        let gatekeeper = crate::tools::gatekeeper::Gatekeeper::new();

        let assembled_v1 = assembler
            .assemble(&state, &ephemeral, &gatekeeper, false)
            .await
            .expect("assemble v1");
        assert!(assembled_v1.contains("I am version 1."));

        tx.send(Arc::from("I am version 2."))
            .expect("send updated identity");

        let assembled_v2 = assembler
            .assemble(&state, &ephemeral, &gatekeeper, false)
            .await
            .expect("assemble v2");
        assert!(assembled_v2.contains("I am version 2."));
        assert_ne!(assembled_v1, assembled_v2);
    }

    #[tokio::test]
    async fn test_moltbook_overlay_is_opt_in() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = "test_workspace";
        let core_dir = dir.path().join(workspace).join("00_Invariants");
        tokio::fs::create_dir_all(&core_dir)
            .await
            .expect("create core dir");
        tokio::fs::write(core_dir.join("Moltbook.md"), "Moltbook-only session rules")
            .await
            .expect("write moltbook overlay");
        let (_tx, rx) = tokio::sync::watch::channel(Arc::from("Base identity"));
        let assembler = ContextAssembler::new(dir.path(), workspace, rx, 3500);
        let ephemeral = EphemeralMemory::new(workspace.to_string());
        let state = AgentState::Idle;
        let gatekeeper = crate::tools::gatekeeper::Gatekeeper::new();

        let base = assembler
            .assemble(&state, &ephemeral, &gatekeeper, false)
            .await
            .expect("assemble base");
        assert!(base.contains("Base identity"));
        assert!(!base.contains("Moltbook-only session rules"));

        let with_overlay = assembler
            .assemble(&state, &ephemeral, &gatekeeper, true)
            .await
            .expect("assemble with overlay");
        assert!(with_overlay.contains("Base identity"));
        assert!(with_overlay.contains("Moltbook-only session rules"));
        assert!(
            with_overlay.contains("Depth rule"),
            "Moltbook overlay should append strict protocol appendix"
        );
        assert!(
            with_overlay.contains("Submolts & curiosity"),
            "Moltbook appendix should cover submolt exploration depth"
        );
    }

    #[tokio::test]
    async fn test_assemble_slim_tool_map_omits_parameters_in_defs() {
        let vault_root = std::path::Path::new("/tmp/unused_for_snapshot_test");
        let workspace = "test_workspace";
        let (_tx, rx) = tokio::sync::watch::channel(Arc::from("I am the test agent."));
        let assembler = ContextAssembler::new(vault_root, workspace, rx, 3500);
        let ephemeral = EphemeralMemory::new(workspace.to_string());
        let state = AgentState::Chat;
        let mut gatekeeper = crate::tools::gatekeeper::Gatekeeper::new();
        gatekeeper.register(Arc::new(crate::tools::system::health::SystemHealthTool {
            config: Arc::new(crate::config::AppConfig::default()),
        }));
        let assembled = assembler
            .assemble_slim_tool_map(&state, &ephemeral, &gatekeeper, None, &[], false)
            .await
            .expect("assemble_slim");
        assert!(
            assembled.contains("Slim tool mode"),
            "expected slim preamble"
        );
        assert!(
            assembled.contains("[FCP_TOOL_PHRASE_MAP]"),
            "expected phrase map"
        );
        assert!(
            !assembled.contains("\"parameters\""),
            "slim prompt must not embed JSON parameter schemas"
        );
    }

    #[test]
    fn tools_need_session_reference_time_db_and_calendar_prefix() {
        let db =
            serde_json::json!({"function": {"name": "db:find_connections", "description": ""}});
        let cal = serde_json::json!({"function": {"name": "calendar:list", "description": ""}});
        let vault = serde_json::json!({"function": {"name": "vault:read", "description": ""}});
        assert!(super::tools_need_session_reference_time(&[db.clone()]));
        assert!(super::tools_need_session_reference_time(&[cal.clone()]));
        assert!(!super::tools_need_session_reference_time(&[vault.clone()]));
        assert!(super::tools_need_session_reference_time(&[vault, cal]));
    }

    #[test]
    fn append_session_reference_time_inserts_block_for_calendar_tool() {
        let tools =
            vec![serde_json::json!({"function": {"name": "calendar:create", "description": ""}})];
        let out = super::append_session_reference_time_if_needed("PREAMBLE".into(), &tools);
        assert!(out.contains("[SESSION_REFERENCE_TIME]"));
        assert!(out.contains("calendar:list"));
        assert!(out.starts_with("PREAMBLE"));
    }

    /// Empty `offered` in slim mode means the assembler keeps **all** allowed tools in the
    /// prompt; llama.cpp GBNF must fall back to session grammar (see `step.rs`).
    #[test]
    fn empty_offered_keeps_full_tool_roster_for_slim_assembly_contract() {
        let a = serde_json::json!({"type":"function","function":{"name":"tool_a","description":"da","parameters":{}}});
        let b = serde_json::json!({"type":"function","function":{"name":"tool_b","description":"db","parameters":{}}});
        let allowed = vec![a, b];
        let out = super::filter_tools_by_offered_order(allowed, &[]);
        assert_eq!(out.len(), 2);
    }
}
