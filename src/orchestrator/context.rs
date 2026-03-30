use std::path::PathBuf;
use crate::executive::error::Result;
use crate::memory::ephemeral::EphemeralMemory;
use crate::orchestrator::state::AgentState;

pub struct ContextAssembler {
    pub core_dir: PathBuf,
}

impl ContextAssembler {
    pub fn new(vault_root: &std::path::Path, workspace: &str) -> Self {
        Self { core_dir: vault_root.join(workspace).join("00_Core") }
    }

    /// Reads Identity.md and formats the Ephemeral cache into a single string.
    /// CRITICAL: `ephemeral.cache` is an async moka cache. You must iterate it safely.
    pub async fn assemble(&self, state: &AgentState, ephemeral: &EphemeralMemory) -> Result<String> {
        let identity_path = self.core_dir.join("Identity.md");
        let identity = tokio::fs::read_to_string(&identity_path).await?;

        let mut cache_contents = String::new();
        // Since `cache` is an async cache, `iter()` provides a synchronous iterator over the current snapshot.
        for (key, value) in ephemeral.cache.iter() {
            cache_contents.push_str(&format!("--- Cache Key: {} ---\n{}\n\n", key, value.data));
        }

        Ok(format!("State: {:?}\n\nIdentity:\n{}\n\nEphemeral Cache:\n{}", state, identity, cache_contents))
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
        let assembled = assembler.assemble(&state, &ephemeral).await.unwrap();
        
        assert!(assembled.contains("I am the test agent."));
        assert!(assembled.contains("test_key"));
        assert!(assembled.contains("test_value_data"));
        assert!(assembled.contains("Idle"));
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
        
        let assembled_v1 = assembler.assemble(&state, &ephemeral).await.unwrap();
        assert!(assembled_v1.contains("I am version 1."));
        
        // Mutate Identity.md
        fs::write(&identity_path, "I am version 2.").await.unwrap();
        
        let assembled_v2 = assembler.assemble(&state, &ephemeral).await.unwrap();
        assert!(assembled_v2.contains("I am version 2."));
        assert!(assembled_v1 != assembled_v2);
    }
}
