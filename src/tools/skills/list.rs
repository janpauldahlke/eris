use std::path::PathBuf;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::executive::error::{FcpError, Result};
use crate::skills::list_vault_skills;
use crate::tools::traits::Tool;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SkillsListArgs {}

pub struct SkillsListTool {
    pub workspace_root: PathBuf,
}

#[async_trait]
impl Tool for SkillsListTool {
    fn name(&self) -> &'static str {
        "skills:list"
    }

    fn description(&self) -> &'static str {
        "List available skill metadata from 10_Topology/skills (id, title, priority, triggers)."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(SkillsListArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let _args: SkillsListArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let skills = list_vault_skills(&self.workspace_root).await?;
        let data: Vec<Value> = skills
            .into_iter()
            .map(|s| {
                json!({
                    "id": s.id,
                    "title": s.title,
                    "priority": match s.priority {
                        crate::skills::SkillPriority::Mandatory => "mandatory",
                        crate::skills::SkillPriority::Conditional => "conditional",
                    },
                    "triggers": s.triggers,
                })
            })
            .collect();
        serde_json::to_string_pretty(&json!({ "skills": data }))
            .map_err(|e| FcpError::EngineFault(format!("skills:list serialization failed: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn list_returns_seeded_skills() {
        let dir = tempdir().expect("tempdir");
        crate::skills::seed_runtime_skills(dir.path())
            .await
            .expect("seed");
        let tool = SkillsListTool {
            workspace_root: dir.path().to_path_buf(),
        };
        let out = tool.execute(json!({})).await.expect("list");
        assert!(out.contains("mail-recipient-verify"));
        assert!(out.contains("db-connections-recovery"));
    }
}
