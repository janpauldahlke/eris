use std::path::PathBuf;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::executive::error::{FcpError, Result};
use crate::skills::{SkillCreateInput, SkillPriority, create_or_update_vault_skill};
use crate::tools::traits::Tool;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillsCreatePriorityArg {
    Mandatory,
    Conditional,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SkillsCreateArgs {
    pub id: String,
    pub title: String,
    pub priority: SkillsCreatePriorityArg,
    pub triggers: Vec<String>,
    pub body: String,
    #[serde(default)]
    pub overwrite: bool,
}

pub struct SkillsCreateTool {
    pub workspace_root: PathBuf,
}

#[async_trait]
impl Tool for SkillsCreateTool {
    fn name(&self) -> &'static str {
        "skills:create"
    }

    fn description(&self) -> &'static str {
        "Create or overwrite a skill file in 10_Topology/skills with strict validation."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(SkillsCreateArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: SkillsCreateArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let receipt = create_or_update_vault_skill(
            &self.workspace_root,
            SkillCreateInput {
                id: args.id,
                title: args.title,
                priority: match args.priority {
                    SkillsCreatePriorityArg::Mandatory => SkillPriority::Mandatory,
                    SkillsCreatePriorityArg::Conditional => SkillPriority::Conditional,
                },
                triggers: args.triggers,
                body: args.body,
                overwrite: args.overwrite,
            },
        )
        .await?;
        serde_json::to_string_pretty(&json!({
            "status": "ok",
            "relative_path": receipt.relative_path,
            "overwritten": receipt.overwritten,
            "skill": {
                "id": receipt.skill.id,
                "title": receipt.skill.title,
                "priority": match receipt.skill.priority {
                    SkillPriority::Mandatory => "mandatory",
                    SkillPriority::Conditional => "conditional",
                },
                "triggers": receipt.skill.triggers,
                "body": receipt.skill.body,
            }
        }))
        .map_err(|e| FcpError::EngineFault(format!("skills:create serialization failed: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test(flavor = "current_thread")]
    async fn create_success() {
        let dir = tempdir().expect("tempdir");
        let tool = SkillsCreateTool {
            workspace_root: dir.path().to_path_buf(),
        };
        let out = tool.execute(json!({
            "id": "sample-skill",
            "title": "Sample",
            "priority": "mandatory",
            "triggers": ["skills:list"],
            "body": "Use this.",
            "overwrite": false
        })).await.expect("create");
        assert!(out.contains("\"status\": \"ok\""));
        assert!(out.contains("sample-skill"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn duplicate_rejected_without_overwrite() {
        let dir = tempdir().expect("tempdir");
        let tool = SkillsCreateTool {
            workspace_root: dir.path().to_path_buf(),
        };
        let payload = json!({
            "id": "sample-skill",
            "title": "Sample",
            "priority": "mandatory",
            "triggers": ["skills:list"],
            "body": "Use this.",
            "overwrite": false
        });
        let _ = tool.execute(payload.clone()).await.expect("first");
        let err = tool.execute(payload).await.expect_err("second should fail");
        assert!(err.to_string().contains("already exists"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn overwrite_allowed_with_flag() {
        let dir = tempdir().expect("tempdir");
        let tool = SkillsCreateTool {
            workspace_root: dir.path().to_path_buf(),
        };
        let _ = tool
            .execute(json!({
                "id": "sample-skill",
                "title": "Sample",
                "priority": "mandatory",
                "triggers": ["skills:list"],
                "body": "Use this.",
                "overwrite": false
            }))
            .await
            .expect("first");
        let out = tool
            .execute(json!({
                "id": "sample-skill",
                "title": "Sample Updated",
                "priority": "conditional",
                "triggers": ["skills:read"],
                "body": "Updated.",
                "overwrite": true
            }))
            .await
            .expect("overwrite");
        assert!(out.contains("\"overwritten\": true"));
        assert!(out.contains("Sample Updated"));
    }
}
