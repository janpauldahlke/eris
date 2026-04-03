use std::path::PathBuf;
use std::sync::Arc;

use crate::executive::error::Result;
use crate::memory::ephemeral::EphemeralMemory;
use crate::orchestrator::state::AgentState;
use crate::tools::gatekeeper::Gatekeeper;

pub struct ContextAssembler {
    pub core_dir: PathBuf,
    identity: tokio::sync::watch::Receiver<Arc<str>>,
}

impl ContextAssembler {
    pub fn new(
        vault_root: &std::path::Path,
        workspace: &str,
        identity: tokio::sync::watch::Receiver<Arc<str>>,
    ) -> Self {
        Self {
            core_dir: vault_root.join(workspace).join("00_Core"),
            identity,
        }
    }

    fn identity_text(&self) -> String {
        (*self.identity.borrow()).as_ref().to_string()
    }

    /// Reads Identity.md and formats the Ephemeral cache into a single string.
    /// CRITICAL: `ephemeral.cache` is an async moka cache. You must iterate it safely.
    pub async fn assemble(&self, state: &AgentState, _ephemeral: &EphemeralMemory, gatekeeper: &Gatekeeper) -> Result<String> {
        let identity_content = self.identity_text();
        let allowed_tools = gatekeeper.get_allowed_tools(state);
        Self::build_tool_prompt(identity_content, allowed_tools)
    }

    pub async fn assemble_with_selected_tools(
        &self,
        state: &AgentState,
        _ephemeral: &EphemeralMemory,
        gatekeeper: &Gatekeeper,
        selected_tools: &[String],
    ) -> Result<String> {
        let identity_content = self.identity_text();

        let allowed_tools = gatekeeper
            .get_allowed_tools(state)
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

        Self::build_tool_prompt(identity_content, allowed_tools)
    }

    fn build_tool_prompt(identity_content: String, allowed_tools: Vec<serde_json::Value>) -> Result<String> {
        tracing::info!(tool_count = allowed_tools.len(), "Tools included in assembled prompt");
        let tools_schema_string = serde_json::to_string_pretty(&allowed_tools)
            .unwrap_or_else(|_| "[]".to_string());

        let system_prompt = format!(
            "{identity}\n\n\
            You are inside a strict agent loop. Reply with ONE valid JSON object only.\n\
            No markdown. No prose outside JSON. No code fences.\n\n\
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
            Available tools for current state:\n{tools}\n\n\
            Memory lifecycle rules (follow exactly):\n\
            - memory:stage creates temporary entries in ephemeral memory and returns a staged_id.\n\
            - Staged entries EXPIRE on TTL; they do not auto-promote.\n\
            - Use memory:staged_list to inspect staged entries before committing.\n\
            - Prefer memory:commit with staged_id for single-item persistence.\n\
            - Use memory:commit_all for best-effort bulk persistence.\n\
            - Web fetch staging (tags web_artifact): committing does NOT write markdown to disk; semantic chunks were stored at fetch time.\n\n\
            Vault taxonomy — when using memory:stage, include tags from the correct category:\n\
            - person, contact, people → stored in 30_Persons/\n\
            - user, preference, about_me → stored in 40_User/\n\
            - semantic, knowledge, api, reference, concept → stored in 20_Semantic/\n\
            - Everything else → stored in 10_Episodic/\n\
            The tags you provide at stage time determine where content is physically stored on disk.",
            identity = identity_content,
            tools = tools_schema_string
        );

        Ok(system_prompt)
    }

    /// Builds a tool-free conversational prompt.
    /// The LLM responds naturally; its `thought` field is later fed to the
    /// ToolRouter for semantic gating.
    pub async fn assemble_conversational(&self, _ephemeral: &EphemeralMemory) -> Result<String> {
        let identity_content = self.identity_text();

        let system_prompt = format!(
            "{identity}\n\n\
            Reply with ONE valid JSON object only. No markdown. No code fences.\n\n\
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
            identity = identity_content,
        );

        Ok(system_prompt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_assembler_reads_identity_snapshot() {
        let vault_root = std::path::Path::new("/tmp/unused_for_snapshot_test");
        let workspace = "test_workspace";
        let (_tx, rx) = tokio::sync::watch::channel(Arc::from("I am the test agent."));
        let assembler = ContextAssembler::new(vault_root, workspace, rx);
        let ephemeral = EphemeralMemory::new(workspace.to_string());

        ephemeral
            .insert("test_key", "test_value_data", vec![], 60)
            .await
            .expect("insert");

        let state = AgentState::Idle;
        let gatekeeper = crate::tools::gatekeeper::Gatekeeper::new();
        let assembled = assembler
            .assemble(&state, &ephemeral, &gatekeeper)
            .await
            .expect("assemble");

        assert!(assembled.contains("I am the test agent."));
        assert!(assembled.contains("Reply with ONE valid JSON object only"));
        assert!(assembled.contains("\"status\": \"Task|Reflect|Idle\""));
    }

    #[tokio::test]
    async fn test_assembler_identity_hot_reload_via_watch() {
        let vault_root = std::path::Path::new("/tmp/unused_for_snapshot_test");
        let workspace = "test_workspace";
        let (tx, rx) = tokio::sync::watch::channel(Arc::from("I am version 1."));
        let assembler = ContextAssembler::new(vault_root, workspace, rx);
        let ephemeral = EphemeralMemory::new(workspace.to_string());
        let state = AgentState::Idle;
        let gatekeeper = crate::tools::gatekeeper::Gatekeeper::new();

        let assembled_v1 = assembler
            .assemble(&state, &ephemeral, &gatekeeper)
            .await
            .expect("assemble v1");
        assert!(assembled_v1.contains("I am version 1."));

        tx.send(Arc::from("I am version 2."))
            .expect("send updated identity");

        let assembled_v2 = assembler
            .assemble(&state, &ephemeral, &gatekeeper)
            .await
            .expect("assemble v2");
        assert!(assembled_v2.contains("I am version 2."));
        assert_ne!(assembled_v1, assembled_v2);
    }
}
