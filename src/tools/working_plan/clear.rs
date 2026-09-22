use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;

use super::{archive_and_clear, clear as clear_file};
use crate::executive::error::{FcpError, Result};
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct PlanClearArgs {
    /// When true (default), copy the plan into `.fcp/tools/working_plan_archive/` before clearing.
    #[serde(default = "default_archive")]
    pub archive: bool,
}

fn default_archive() -> bool {
    true
}

pub struct PlanClearTool {
    pub workspace_root: PathBuf,
}

#[async_trait]
impl Tool for PlanClearTool {
    fn name(&self) -> &'static str {
        "plan:clear"
    }
    fn description(&self) -> &'static str {
        "Clear the active working plan (archive by default). Call when the mission is finished \
         or abandoned so Status UI and plan pinning drop."
    }
    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(PlanClearArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: PlanClearArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        if args.archive {
            match archive_and_clear(&self.workspace_root).await? {
                Some(name) => Ok(format!(
                    "SUCCESS: Working plan archived as {name} and cleared."
                )),
                None => Ok("No working plan set.".to_string()),
            }
        } else if clear_file(&self.workspace_root).await? {
            Ok("SUCCESS: Working plan cleared (not archived).".to_string())
        } else {
            Ok("No working plan set.".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::working_plan::{load, save, PlanStep, PlanStepStatus, WorkingPlan};
    use tempfile::tempdir;

    #[tokio::test(flavor = "current_thread")]
    async fn test_plan_clear_archives_and_removes() -> Result<()> {
        let dir = tempdir().unwrap();
        save(
            dir.path(),
            &WorkingPlan {
                goal: "done mission".into(),
                steps: vec![PlanStep {
                    id: "a".into(),
                    title: "A".into(),
                    status: PlanStepStatus::Done,
                    kind: None,
                }],
                version: 1,
                ..Default::default()
            },
        )
        .await?;
        let tool = PlanClearTool {
            workspace_root: dir.path().to_path_buf(),
        };
        let result = tool.execute(serde_json::json!({})).await?;
        assert!(result.contains("SUCCESS"), "result: {result}");
        assert!(result.contains("archived"), "result: {result}");
        assert!(load(dir.path()).await?.is_none());
        let archive = crate::vault_layout::working_plan_archive_dir(dir.path());
        let mut entries = tokio::fs::read_dir(&archive).await.map_err(FcpError::Io)?;
        let first = entries.next_entry().await.map_err(FcpError::Io)?;
        assert!(first.is_some(), "archive should contain a file");
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_plan_clear_no_file() -> Result<()> {
        let dir = tempdir().unwrap();
        let tool = PlanClearTool {
            workspace_root: dir.path().to_path_buf(),
        };
        let result = tool.execute(serde_json::json!({})).await?;
        assert_eq!(result, "No working plan set.");
        Ok(())
    }
}
