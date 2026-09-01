use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use tokio::fs;

use crate::executive::error::{FcpError, Result};
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct PlanReadArgs {}

pub struct PlanReadTool {
    pub workspace_root: PathBuf,
}

#[async_trait]
impl Tool for PlanReadTool {
    fn name(&self) -> &'static str {
        "plan:read"
    }
    fn description(&self) -> &'static str {
        "Read the full working plan (goal, outcome, steps, scratch) as JSON."
    }
    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(PlanReadArgs)
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        let path = crate::vault_layout::working_plan_json(&self.workspace_root);
        if !path.exists() {
            return Ok("No working plan set.".to_string());
        }
        let content = fs::read_to_string(&path)
            .await
            .map_err(FcpError::Io)?;
        if content.trim().is_empty() {
            return Ok("No working plan set.".to_string());
        }
        // Round-trip through the type so a corrupted file surfaces as a ParseFault,
        // and the model always sees normalized pretty JSON.
        let plan: crate::tools::working_plan::WorkingPlan =
            serde_json::from_str(&content).map_err(FcpError::ParseFault)?;
        Ok(serde_json::to_string_pretty(&plan).map_err(FcpError::ParseFault)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::working_plan::{save, WorkingPlan};
    use tempfile::tempdir;

    #[tokio::test(flavor = "current_thread")]
    async fn test_plan_read_no_file() -> Result<()> {
        let dir = tempdir().unwrap();
        let tool = PlanReadTool {
            workspace_root: dir.path().to_path_buf(),
        };
        let result = tool.execute(serde_json::json!({})).await?;
        assert_eq!(result, "No working plan set.");
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_plan_read_returns_full_json() -> Result<()> {
        let dir = tempdir().unwrap();
        save(
            dir.path(),
            &WorkingPlan {
                goal: "ship it".into(),
                outcome: "tests green".into(),
                steps: vec![crate::tools::working_plan::PlanStep {
                    id: "a".into(),
                    title: "step a".into(),
                    status: crate::tools::working_plan::PlanStepStatus::Active,
                    kind: None,
                }],
                current_step_id: Some("a".into()),
                scratch: "wip".into(),
                updated_at: 1,
                version: 1,
            },
        )
        .await?;
        let tool = PlanReadTool {
            workspace_root: dir.path().to_path_buf(),
        };
        let result = tool.execute(serde_json::json!({})).await?;
        assert!(result.contains("\"goal\": \"ship it\""), "result: {result}");
        assert!(result.contains("\"current_step_id\": \"a\""), "result: {result}");
        assert!(result.contains("\"scratch\": \"wip\""), "result: {result}");
        Ok(())
    }
}
