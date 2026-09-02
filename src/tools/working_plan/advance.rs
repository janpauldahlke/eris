use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{load, maybe_auto_archive_if_complete, save};
use crate::executive::error::{FcpError, Result};
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct PlanAdvanceArgs {
    /// Optional short note appended to scratch when advancing.
    #[serde(default)]
    pub scratch_append: Option<String>,
}

pub struct PlanAdvanceTool {
    pub workspace_root: PathBuf,
}

#[async_trait]
impl Tool for PlanAdvanceTool {
    fn name(&self) -> &'static str {
        "plan:advance"
    }
    fn description(&self) -> &'static str {
        "Mark the current working-plan step done and move current_step_id to the next open step. \
         Prefer this over a partial plan:update when finishing a step. Auto-archives when the \
         mission has no open steps left."
    }
    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(PlanAdvanceArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: PlanAdvanceArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let mut plan = match load(&self.workspace_root).await? {
            Some(plan) => plan,
            None => {
                return Ok(
                    "No working plan set; call plan:set first to create one.".to_string()
                );
            }
        };

        if let Some(extra) = args.scratch_append {
            let extra = extra.trim().to_string();
            if !extra.is_empty() {
                if !plan.scratch.is_empty() {
                    plan.scratch.push('\n');
                }
                plan.scratch.push_str(&extra);
            }
        }

        let (done_id, next_id) = plan.advance_current()?;
        plan.version += 1;
        plan.updated_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        save(&self.workspace_root, &plan).await?;

        let mut msg = match &next_id {
            Some(nid) => {
                let title = plan
                    .steps
                    .iter()
                    .find(|s| &s.id == nid)
                    .map(|s| s.title.trim())
                    .unwrap_or("");
                format!(
                    "SUCCESS: Advanced plan (version {}): marked {done_id} done; current={nid} ({title}).",
                    plan.version
                )
            }
            None => format!(
                "SUCCESS: Advanced plan (version {}): marked {done_id} done; no open steps remain.",
                plan.version
            ),
        };

        if let Some(note) = maybe_auto_archive_if_complete(&self.workspace_root, &plan).await? {
            msg.push_str(&note);
        }
        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::working_plan::{PlanStep, PlanStepStatus, WorkingPlan};
    use tempfile::tempdir;

    async fn seed_two(dir: &std::path::Path) {
        save(
            dir,
            &WorkingPlan {
                goal: "g".into(),
                steps: vec![
                    PlanStep {
                        id: "a".into(),
                        title: "First".into(),
                        status: PlanStepStatus::Active,
                        kind: None,
                    },
                    PlanStep {
                        id: "b".into(),
                        title: "Second".into(),
                        status: PlanStepStatus::Pending,
                        kind: None,
                    },
                ],
                current_step_id: Some("a".into()),
                version: 1,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_plan_advance_moves_pointer() -> Result<()> {
        let dir = tempdir().unwrap();
        seed_two(dir.path()).await;
        let tool = PlanAdvanceTool {
            workspace_root: dir.path().to_path_buf(),
        };
        let result = tool
            .execute(serde_json::json!({ "scratch_append": "first done" }))
            .await?;
        assert!(result.contains("marked a done"), "result: {result}");
        assert!(result.contains("current=b"), "result: {result}");
        let plan = load(dir.path()).await?.unwrap();
        assert_eq!(plan.steps[0].status, PlanStepStatus::Done);
        assert_eq!(plan.steps[1].status, PlanStepStatus::Active);
        assert_eq!(plan.current_step_id.as_deref(), Some("b"));
        assert!(plan.scratch.contains("first done"));
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_plan_advance_last_step_archives() -> Result<()> {
        let dir = tempdir().unwrap();
        save(
            dir.path(),
            &WorkingPlan {
                goal: "g".into(),
                steps: vec![PlanStep {
                    id: "only".into(),
                    title: "Only".into(),
                    status: PlanStepStatus::Active,
                    kind: None,
                }],
                current_step_id: Some("only".into()),
                version: 1,
                ..Default::default()
            },
        )
        .await?;
        let tool = PlanAdvanceTool {
            workspace_root: dir.path().to_path_buf(),
        };
        let result = tool.execute(serde_json::json!({})).await?;
        assert!(result.contains("Mission complete"), "result: {result}");
        assert!(load(dir.path()).await?.is_none());
        Ok(())
    }
}
