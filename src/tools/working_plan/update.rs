use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{
    load, new_step_id, save, PlanStep, PlanStepInput, PlanStepKind, PlanStepStatus, WorkingPlan,
};
use crate::executive::error::{FcpError, Result};
use crate::tools::traits::Tool;

/// Patch for an existing step, addressed by id.
#[derive(Deserialize, JsonSchema)]
pub struct PlanStepPatch {
    pub id: String,
    #[serde(default)]
    pub status: Option<PlanStepStatus>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub kind: Option<PlanStepKind>,
}

#[derive(Deserialize, JsonSchema)]
pub struct PlanUpdateArgs {
    /// Replace the goal.
    #[serde(default)]
    pub goal: Option<String>,
    /// Replace the outcome.
    #[serde(default)]
    pub outcome: Option<String>,
    /// Append to the scratch pad (newline-separated).
    #[serde(default)]
    pub scratch_append: Option<String>,
    /// Move the current-step pointer (must match a step id).
    #[serde(default)]
    pub current_step_id: Option<String>,
    /// Patch existing steps by id.
    #[serde(default)]
    pub steps: Option<Vec<PlanStepPatch>>,
    /// Append new steps.
    #[serde(default)]
    pub steps_add: Option<Vec<PlanStepInput>>,
}

pub struct PlanUpdateTool {
    pub workspace_root: PathBuf,
}

impl PlanUpdateArgs {
    fn is_empty(&self) -> bool {
        self.goal.is_none()
            && self.outcome.is_none()
            && self.scratch_append.is_none()
            && self.current_step_id.is_none()
            && self.steps.is_none()
            && self.steps_add.is_none()
    }
}

fn apply_patch(plan: &mut WorkingPlan, args: PlanUpdateArgs) -> Result<Vec<String>> {
    let mut changed: Vec<String> = Vec::new();

    if let Some(goal) = args.goal {
        plan.goal = goal;
        changed.push("goal".into());
    }
    if let Some(outcome) = args.outcome {
        plan.outcome = outcome;
        changed.push("outcome".into());
    }
    if let Some(extra) = args.scratch_append {
        let extra = extra.trim().to_string();
        if !extra.is_empty() {
            if !plan.scratch.is_empty() {
                plan.scratch.push('\n');
            }
            plan.scratch.push_str(&extra);
            changed.push("scratch".into());
        }
    }
    for input in args.steps_add.unwrap_or_default() {
        let id = input
            .id
            .filter(|i| !i.trim().is_empty())
            .unwrap_or_else(new_step_id);
        if plan.steps.iter().any(|s| s.id == id) {
            return Err(FcpError::SchemaViolation(format!(
                "plan:update: steps_add id {id:?} already exists"
            )));
        }
        plan.steps.push(PlanStep {
            id: id.clone(),
            title: input.title,
            status: input.status.unwrap_or(PlanStepStatus::Pending),
            kind: input.kind,
        });
        changed.push(format!("step+ {id}"));
    }
    for patch in args.steps.unwrap_or_default() {
        let Some(idx) = plan.steps.iter().position(|s| s.id == patch.id) else {
            return Err(FcpError::SchemaViolation(format!(
                "plan:update: step id {:?} not found (known: {})",
                patch.id,
                plan.steps.iter().map(|s| s.id.as_str()).collect::<Vec<_>>().join(", ")
            )));
        };
        let step = &mut plan.steps[idx];
        if let Some(status) = patch.status {
            step.status = status;
            changed.push(format!("{}={}", step.id, status.as_str()));
        }
        if let Some(title) = patch.title {
            step.title = title;
        }
        if let Some(kind) = patch.kind {
            step.kind = Some(kind);
        }
    }
    if let Some(id) = args.current_step_id {
        if !plan.steps.iter().any(|s| s.id == id) {
            return Err(FcpError::SchemaViolation(format!(
                "plan:update: current_step_id {id:?} does not match any step"
            )));
        }
        if plan.current_step_id.as_deref() != Some(id.as_str()) {
            plan.current_step_id = Some(id);
            changed.push("current".into());
        }
    }
    Ok(changed)
}

#[async_trait]
impl Tool for PlanUpdateTool {
    fn name(&self) -> &'static str {
        "plan:update"
    }
    fn description(&self) -> &'static str {
        "Patch the working plan: step statuses/titles/kinds, append steps, goal/outcome, \
         scratch_append, and current_step_id. Call after each significant step so the plan \
         survives context condensation."
    }
    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(PlanUpdateArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: PlanUpdateArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        if args.is_empty() {
            return Ok(
                "Nothing to update: provide goal, outcome, scratch_append, current_step_id, \
                 steps, or steps_add."
                    .to_string(),
            );
        }

        let mut plan = match load(&self.workspace_root).await? {
            Some(plan) => plan,
            None => {
                return Ok(
                    "No working plan set; call plan:set first to create one.".to_string()
                );
            }
        };

        let changed = apply_patch(&mut plan, args)?;
        plan.version += 1;
        plan.updated_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        save(&self.workspace_root, &plan).await?;

        let summary = if changed.is_empty() {
            "no changes".to_string()
        } else {
            changed.join(", ")
        };
        Ok(format!(
            "SUCCESS: Working plan updated (version {}): {}.",
            plan.version, summary
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    async fn seed(dir: &std::path::Path) -> WorkingPlan {
        let plan = WorkingPlan {
            goal: "Ship the slice".into(),
            outcome: "tests green".into(),
            steps: vec![
                PlanStep {
                    id: "a".into(),
                    title: "Step A".into(),
                    status: PlanStepStatus::Active,
                    kind: None,
                },
                PlanStep {
                    id: "b".into(),
                    title: "Step B".into(),
                    status: PlanStepStatus::Pending,
                    kind: None,
                },
            ],
            current_step_id: Some("a".into()),
            scratch: "first note".into(),
            updated_at: 1,
            version: 3,
        };
        save(dir, &plan).await.unwrap();
        plan
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_plan_update_marks_step_done_and_advances() -> Result<()> {
        let dir = tempdir().unwrap();
        seed(dir.path()).await;
        let tool = PlanUpdateTool {
            workspace_root: dir.path().to_path_buf(),
        };
        let result = tool
            .execute(serde_json::json!({
                "steps": [{ "id": "a", "status": "done" }],
                "current_step_id": "b",
                "scratch_append": "step a finished"
            }))
            .await?;
        assert!(result.starts_with("SUCCESS: Working plan updated (version 4)"), "result: {result}");

        let plan = load(dir.path()).await?.unwrap();
        assert_eq!(plan.version, 4);
        assert_eq!(plan.steps[0].status, PlanStepStatus::Done);
        assert_eq!(plan.current_step_id.as_deref(), Some("b"));
        assert_eq!(plan.scratch, "first note\nstep a finished");
        assert_eq!(plan.goal, "Ship the slice");
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_plan_update_appends_steps_and_goal() -> Result<()> {
        let dir = tempdir().unwrap();
        seed(dir.path()).await;
        let tool = PlanUpdateTool {
            workspace_root: dir.path().to_path_buf(),
        };
        let result = tool
            .execute(serde_json::json!({
                "goal": "Revised goal",
                "steps_add": [{ "title": "Step C", "kind": "validate" }]
            }))
            .await?;
        assert!(result.contains("version 4"), "result: {result}");

        let plan = load(dir.path()).await?.unwrap();
        assert_eq!(plan.goal, "Revised goal");
        assert_eq!(plan.steps.len(), 3);
        assert_eq!(plan.steps[2].title, "Step C");
        assert_eq!(plan.steps[2].kind, Some(PlanStepKind::Validate));
        assert_eq!(plan.steps[2].status, PlanStepStatus::Pending);
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_plan_update_unknown_step_id_rejected() -> Result<()> {
        let dir = tempdir().unwrap();
        seed(dir.path()).await;
        let tool = PlanUpdateTool {
            workspace_root: dir.path().to_path_buf(),
        };
        let err = tool
            .execute(serde_json::json!({
                "steps": [{ "id": "zzz", "status": "done" }]
            }))
            .await
            .expect_err("unknown step id must be rejected");
        match err {
            FcpError::SchemaViolation(msg) => {
                assert!(msg.contains("zzz"), "msg: {msg}");
            }
            other => panic!("expected SchemaViolation, got {other:?}"),
        }
        // Rejected update must not bump the version.
        let plan = load(dir.path()).await?.unwrap();
        assert_eq!(plan.version, 3);
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_plan_update_without_plan_is_soft_message() -> Result<()> {
        let dir = tempdir().unwrap();
        let tool = PlanUpdateTool {
            workspace_root: dir.path().to_path_buf(),
        };
        let result = tool
            .execute(serde_json::json!({ "goal": "g" }))
            .await?;
        assert!(result.contains("plan:set first"), "result: {result}");
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_plan_update_empty_args_is_noop_message() -> Result<()> {
        let dir = tempdir().unwrap();
        seed(dir.path()).await;
        let tool = PlanUpdateTool {
            workspace_root: dir.path().to_path_buf(),
        };
        let result = tool.execute(serde_json::json!({})).await?;
        assert!(result.contains("Nothing to update"), "result: {result}");
        let plan = load(dir.path()).await?.unwrap();
        assert_eq!(plan.version, 3);
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_plan_update_bad_current_step_id_rejected() -> Result<()> {
        let dir = tempdir().unwrap();
        seed(dir.path()).await;
        let tool = PlanUpdateTool {
            workspace_root: dir.path().to_path_buf(),
        };
        let err = tool
            .execute(serde_json::json!({ "current_step_id": "nope" }))
            .await
            .expect_err("unknown current_step_id must be rejected");
        assert!(matches!(err, FcpError::SchemaViolation(_)));
        Ok(())
    }
}
