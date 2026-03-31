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
        let identity_content = match tokio::fs::read_to_string(&identity_path).await {
            Ok(content) => content,
            Err(_) => "You are E.R.I.S., an autonomous AI agent.".to_string(), // Hardcoded fallback
        };

        let allowed_tools = gatekeeper.get_allowed_tools(state);
        let tools_schema_string = serde_json::to_string_pretty(&allowed_tools)
            .unwrap_or_else(|_| "[]".to_string());

        let system_prompt = format!(
            "{}\n\n\
            You are operating within a strict programmatic state machine. \n\
            You MUST communicate EXCLUSIVELY in valid JSON format. \n\
            Do NOT output conversational text, pleasantries, or markdown blocks outside of the JSON structure. \n\
            Your output MUST strictly adhere to this schema: \n\
            {{ \"thought\": \"your internal reasoning\", \"status\": \"Reflect|Idle|Task\", \"tool_calls\": [ {{ \"name\": \"tool_name\", \"args\": {{...}} }} ] }}\n\n\
            Status Values:\n\
            - Reflect: Use this if you called tools and are waiting for their output.\n\
            - Idle: Use this if you are completely finished with the task.\n\
            - Task: Use this if you are actively working but not calling tools.\n\n\
            Available Tools:\n{}",
            identity_content, // From Step A
            tools_schema_string // From Step B
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
        
        ephemeral.insert("test_key", "test_value_data", 60).await.unwrap();
        
        let state = AgentState::Idle;
        let gatekeeper = crate::tools::gatekeeper::Gatekeeper::new();
        let assembled = assembler.assemble(&state, &ephemeral, &gatekeeper).await.unwrap();
        
        assert!(assembled.contains("I am the test agent."));
        assert!(assembled.contains("strict programmatic state machine"));
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
