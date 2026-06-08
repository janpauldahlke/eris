use std::path::PathBuf;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::executive::error::{FcpError, Result};
use crate::skills::{SkillPriority, load_vault_skill_by_id};
use crate::tools::traits::Tool;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SkillsReadArgs {
    pub id: String,
}

pub struct SkillsReadTool {
    pub workspace_root: PathBuf,
}

#[async_trait]
impl Tool for SkillsReadTool {
    fn name(&self) -> &'static str {
        "skills:read"
    }

    fn description(&self) -> &'static str {
        "Read one skill by id from 10_Topology/skills and return structured fields."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(SkillsReadArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: SkillsReadArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let id = args.id.trim();
        if id.is_empty() {
            return Err(FcpError::SchemaViolation("id cannot be empty".to_string()));
        }
        let Some(skill) = load_vault_skill_by_id(&self.workspace_root, id).await? else {
            return Err(FcpError::ToolFault {
                tool_name: self.name().to_string(),
                reason: format!("Skill not found in vault: {}", id),
            });
        };
        let out = json!({
            "id": skill.id,
            "title": skill.title,
            "priority": match skill.priority {
                SkillPriority::Mandatory => "mandatory",
                SkillPriority::Conditional => "conditional",
            },
            "triggers": skill.triggers,
            "body": skill.body,
        });
        serde_json::to_string_pretty(&out)
            .map_err(|e| FcpError::EngineFault(format!("skills:read serialization failed: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test(flavor = "current_thread")]
    async fn read_returns_structured_skill() {
        let dir = tempdir().expect("tempdir");
        crate::skills::seed_runtime_skills(dir.path())
            .await
            .expect("seed");
        let tool = SkillsReadTool {
            workspace_root: dir.path().to_path_buf(),
        };
        let out = tool
            .execute(json!({ "id": "mail-recipient-verify" }))
            .await
            .expect("read");
        assert!(out.contains("\"id\": \"mail-recipient-verify\""));
        assert!(out.contains("\"body\""));
    }
}
