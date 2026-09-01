use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{new_step_id, save, PlanStep, PlanStepInput, PlanStepStatus, WorkingPlan};
use crate::executive::error::{FcpError, Result};
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct PlanSetArgs {
    pub goal: String,
    #[serde(default)]
    pub outcome: Option<String>,
    /// Ordered steps. Each step gets an id (auto-generated when omitted).
    #[serde(default)]
    pub steps: Vec<PlanStepInput>,
    /// Replaces any previous scratch content.
    #[serde(default)]
    pub scratch: Option<String>,
    /// Must match a step id; defaults to the first step when omitted.
    #[serde(default)]
    pub current_step_id: Option<String>,
}

pub struct PlanSetTool {
    pub workspace_root: PathBuf,
}

impl PlanSetTool {
    fn build_plan(&self, args: PlanSetArgs) -> Result<WorkingPlan> {
        let mut seen = std::collections::HashSet::new();
        let mut steps: Vec<PlanStep> = Vec::with_capacity(args.steps.len());
        for input in args.steps {
            let id = input.id.filter(|i| !i.trim().is_empty()).unwrap_or_else(new_step_id);
            if !seen.insert(id.clone()) {
                return Err(FcpError::SchemaViolation(format!(
                    "plan:set: duplicate step id {id:?}"
                )));
            }
            steps.push(PlanStep {
                id,
                title: input.title,
                status: input.status.unwrap_or(PlanStepStatus::Pending),
                kind: input.kind,
            });
        }

        let current_step_id = args
            .current_step_id
            .filter(|c| !c.trim().is_empty())
            .or_else(|| steps.first().map(|s| s.id.clone()));
        if let Some(id) = &current_step_id {
            if !steps.iter().any(|s| &s.id == id) {
                return Err(FcpError::SchemaViolation(format!(
                    "plan:set: current_step_id {id:?} does not match any step"
                )));
            }
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Ok(WorkingPlan {
            goal: args.goal,
            outcome: args.outcome.unwrap_or_default(),
            steps,
            current_step_id,
            scratch: args.scratch.unwrap_or_default(),
            updated_at: now,
            version: 1,
        })
    }
}

#[async_trait]
impl Tool for PlanSetTool {
    fn name(&self) -> &'static str {
        "plan:set"
    }
    fn description(&self) -> &'static str {
        "Replace the entire working plan (goal, outcome, steps, scratch). Call before \
         executing a multi-step or dependent request."
    }
    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(PlanSetArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: PlanSetArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let plan = self.build_plan(args)?;
        save(&self.workspace_root, &plan).await?;
        let step_count = plan.steps.len();
        let current = plan.current_step_id.as_deref().unwrap_or("-");
        Ok(format!("SUCCESS: Working plan set ({step_count} steps, current: {current})."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::working_plan::{load, PlanStepKind};
    use tempfile::tempdir;

    #[tokio::test(flavor = "current_thread")]
    async fn test_plan_set_writes_file() -> Result<()> {
        let dir = tempdir().unwrap();
        let tool = PlanSetTool {
            workspace_root: dir.path().to_path_buf(),
        };
        let args = serde_json::json!({
            "goal": "Ship the slice",
            "outcome": "tests green",
            "steps": [
                { "title": "Implement tools" },
                { "title": "Write tests", "kind": "validate" }
            ],
            "scratch": "starting"
        });
        let result = tool.execute(args).await?;
        assert!(result.starts_with("SUCCESS: Working plan set (2 steps"), "result: {result}");

        let plan = load(dir.path()).await?.expect("plan file should exist");
        assert_eq!(plan.goal, "Ship the slice");
        assert_eq!(plan.version, 1);
        assert!(plan.updated_at > 0);
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].status, PlanStepStatus::Pending);
        assert_eq!(plan.steps[1].kind, Some(PlanStepKind::Validate));
        // current defaults to first step
        assert_eq!(plan.current_step_id.as_deref(), Some(plan.steps[0].id.as_str()));
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_plan_set_replaces_previous_plan() -> Result<()> {
        let dir = tempdir().unwrap();
        let tool = PlanSetTool {
            workspace_root: dir.path().to_path_buf(),
        };
        tool.execute(serde_json::json!({
            "goal": "old",
            "steps": [{ "title": "old step" }]
        }))
        .await?;
        let result = tool
            .execute(serde_json::json!({
                "goal": "new",
                "steps": [{ "title": "new step" }, { "title": "second" }]
            }))
            .await?;
        assert!(result.contains("2 steps"), "result: {result}");
        let plan = load(dir.path()).await?.unwrap();
        assert_eq!(plan.goal, "new");
        assert_eq!(plan.version, 1);
        assert_eq!(plan.steps.len(), 2);
        assert!(plan.scratch.is_empty());
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_plan_set_bad_current_step_id_rejected() -> Result<()> {
        let dir = tempdir().unwrap();
        let tool = PlanSetTool {
            workspace_root: dir.path().to_path_buf(),
        };
        let err = tool
            .execute(serde_json::json!({
                "goal": "g",
                "steps": [{ "id": "a", "title": "A" }],
                "current_step_id": "zzz"
            }))
            .await
            .expect_err("unknown current_step_id must be rejected");
        assert!(
            matches!(err, FcpError::SchemaViolation(_)),
            "unexpected error: {err:?}"
        );
        assert!(load(dir.path()).await?.is_none());
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_plan_set_duplicate_ids_rejected() -> Result<()> {
        let dir = tempdir().unwrap();
        let tool = PlanSetTool {
            workspace_root: dir.path().to_path_buf(),
        };
        let err = tool
            .execute(serde_json::json!({
                "goal": "g",
                "steps": [{ "id": "a", "title": "A" }, { "id": "a", "title": "B" }]
            }))
            .await
            .expect_err("duplicate ids must be rejected");
        assert!(matches!(err, FcpError::SchemaViolation(_)));
        Ok(())
    }
}
