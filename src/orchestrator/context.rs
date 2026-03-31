use std::path::PathBuf;
use crate::executive::error::Result;
use crate::memory::ephemeral::EphemeralMemory;
use crate::orchestrator::state::AgentState;
use crate::tools::gatekeeper::Gatekeeper;

pub struct ContextAssembler {
    pub core_dir: PathBuf,
}

impl ContextAssembler {
    pub fn new(vault_root: &std::path::Path, workspace: &str) -> Self {
        Self { core_dir: vault_root.join(workspace).join("00_Core") }
    }

    /// Reads Identity.md and formats the Ephemeral cache into a single string.
    /// CRITICAL: `ephemeral.cache` is an async moka cache. You must iterate it safely.
    pub async fn assemble(&self, state: &AgentState, _ephemeral: &EphemeralMemory, gatekeeper: &Gatekeeper) -> Result<String> {
        let identity_path = self.core_dir.join("Identity.md");
        tracing::debug!(path = %identity_path.display(), "Loading identity file");
        let identity_content = match tokio::fs::read_to_string(&identity_path).await {
            Ok(content) => {
                tracing::info!(len = content.len(), "Identity loaded from vault");
                content
            }
            Err(e) => {
                tracing::warn!(path = %identity_path.display(), error = %e, "Identity file not found, using hardcoded fallback");
                "You are E.R.I.S., an autonomous AI agent.".to_string()
            }
        };

        let allowed_tools = gatekeeper.get_allowed_tools(state);
        tracing::info!(state = ?state, tool_count = allowed_tools.len(), "Tools available for current state");
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
            }}\n\n\
            Status rules (follow exactly):\n\
            1) Task: use when continuing internal work/planning. tool_calls MUST be [].\n\
            2) Reflect: use ONLY when calling one or more tools now. tool_calls MUST be non-empty.\n\
            3) Idle: use when done and waiting for user input. tool_calls MUST be [].\n\
            4) In Idle, message_to_user MUST be a non-empty user-facing reply.\n\
            5) If no tool is needed, NEVER choose Reflect.\n\n\
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
        let identity_path = self.core_dir.join("Identity.md");
        let identity_content = match tokio::fs::read_to_string(&identity_path).await {
            Ok(c) => c,
            Err(_) => "You are E.R.I.S., an autonomous AI agent.".to_string(),
        };

        let system_prompt = format!(
            "{identity}\n\n\
            Reply with ONE valid JSON object only. No markdown. No code fences.\n\n\
            JSON shape:\n\
            {{\n\
              \"thought\": \"your internal reasoning (never shown to user)\",\n\
              \"message_to_user\": \"your helpful reply\"\n\
            }}\n\n\
            Rules:\n\
            1) thought is internal-only, used by the runtime for routing.\n\
            2) message_to_user MUST always be a non-empty string.\n\
            3) Answer the user directly, conversationally, and helpfully.",
            identity = identity_content,
        );

        Ok(system_prompt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use tokio::fs;

    #[tokio::test]
    async fn test_assembler_reads_identity_and_cache() {
        let temp_dir = tempdir().unwrap();
        let vault_root = temp_dir.path();
        let workspace = "test_workspace";
        
        // Setup 00_Core directory and Identity.md
        let core_dir = vault_root.join(workspace).join("00_Core");
        fs::create_dir_all(&core_dir).await.unwrap();
        let identity_path = core_dir.join("Identity.md");
        fs::write(&identity_path, "I am the test agent.").await.unwrap();

        let assembler = ContextAssembler::new(vault_root, workspace);
        let ephemeral = EphemeralMemory::new(workspace.to_string());
        
        ephemeral.insert("test_key", "test_value_data", vec![], 60).await.unwrap();
        
        let state = AgentState::Idle;
        let gatekeeper = crate::tools::gatekeeper::Gatekeeper::new();
        let assembled = assembler.assemble(&state, &ephemeral, &gatekeeper).await.unwrap();
        
        assert!(assembled.contains("I am the test agent."));
        assert!(assembled.contains("Reply with ONE valid JSON object only"));
        assert!(assembled.contains("\"status\": \"Task|Reflect|Idle\""));
    }

    #[tokio::test]
    async fn test_assembler_identity_hot_reload() {
        let temp_dir = tempdir().unwrap();
        let vault_root = temp_dir.path();
        let workspace = "test_workspace";
        
        // Setup 00_Core directory and Identity.md
        let core_dir = vault_root.join(workspace).join("00_Core");
        fs::create_dir_all(&core_dir).await.unwrap();
        let identity_path = core_dir.join("Identity.md");
        fs::write(&identity_path, "I am version 1.").await.unwrap();

        let assembler = ContextAssembler::new(vault_root, workspace);
        let ephemeral = EphemeralMemory::new(workspace.to_string());
        let state = AgentState::Idle;
        let gatekeeper = crate::tools::gatekeeper::Gatekeeper::new();
        
        let assembled_v1 = assembler.assemble(&state, &ephemeral, &gatekeeper).await.unwrap();
        assert!(assembled_v1.contains("I am version 1."));
        
        // Mutate Identity.md
        fs::write(&identity_path, "I am version 2.").await.unwrap();
        
        let assembled_v2 = assembler.assemble(&state, &ephemeral, &gatekeeper).await.unwrap();
        assert!(assembled_v2.contains("I am version 2."));
        assert!(assembled_v1 != assembled_v2);
    }
}
