use std::sync::Arc;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::executive::error::{FcpError, Result};
use crate::tools::traits::Tool;
use crate::memory::semantic::SemanticBrain;
use crate::memory::ephemeral::EphemeralMemory;

#[derive(Deserialize, JsonSchema, PartialEq, Debug, Clone)]
#[serde(rename_all = "PascalCase")]
pub enum TargetDomain {
    Semantic,
    Episodic,
}

#[derive(Deserialize, JsonSchema)]
pub struct MemoryCommitArgs {
    pub tag: String,
    pub target_domain: TargetDomain,
}

pub struct MemoryCommitTool {
    pub workspace_root: std::path::PathBuf,
    pub semantic: Arc<SemanticBrain>,
    pub ephemeral: Arc<EphemeralMemory>,
}

#[async_trait]
impl Tool for MemoryCommitTool {
    fn name(&self) -> &'static str {
        "memory:commit"
    }

    fn description(&self) -> &'static str {
        "Pulls moka cache entries for tag, writes them to physical vault or semantic vector db, and invalidates keys."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(MemoryCommitArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: MemoryCommitArgs = serde_json::from_value(args)
            .map_err(FcpError::ParseFault)?;

        let content = self.ephemeral.get(&args.tag).await
            .ok_or_else(|| FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: format!("No staged memory found for tag: {}", args.tag),
            })?;

        match args.target_domain {
            TargetDomain::Semantic => {
                self.semantic.upsert(&content, vec![args.tag.clone()]).await?;
            }
            TargetDomain::Episodic => {
                // For now, fallback to basic file write for episodic or simply append to a markdown file
                let path = self.workspace_root.join(format!("{}.md", args.tag));
                let mut existing = String::new();
                if path.exists() {
                    existing = tokio::fs::read_to_string(&path).await
                        .map_err(|e| FcpError::WorkspaceFault { workspace: args.tag.clone(), reason: e.to_string() })?;
                }
                existing.push_str(&format!("\n\n{}", content));
                tokio::fs::write(&path, existing).await
                    .map_err(|e| FcpError::WorkspaceFault { workspace: args.tag.clone(), reason: e.to_string() })?;
            }
        }
        
        // Invalidate key after commit
        self.ephemeral.cache.invalidate(&args.tag).await;

        Ok(format!("Successfully committed {} to {:?}", args.tag, args.target_domain))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    // Note: To test this properly, you need mock for SemanticBrain, which is tricky without a live Qdrant.
    // For now, we rely on the type checking and orchestrator integration tests.
}
