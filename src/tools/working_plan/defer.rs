//! `plan:defer` — schedule a working-plan resume alarm (Phase 2A).

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use tokio::sync::mpsc;

use super::resume::{fire_at_from_secs, schedule_plan_resume};
use crate::executive::error::{FcpError, Result};
use crate::tools::clock::{next_wall_alarm_fire_local, MAX_TIMER_MINUTES};
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct PlanDeferArgs {
    /// Relative wake in N minutes (1..=1440). XOR with `hour`+`minute`.
    pub minutes: Option<u32>,
    /// Wall-clock hour (0..=23) with `minute`. XOR with `minutes`.
    pub hour: Option<u8>,
    pub minute: Option<u8>,
    /// Optional short note shown on wake (defaults to "Continue working plan").
    #[serde(default)]
    pub note: Option<String>,
}

pub struct PlanDeferTool {
    pub workspace_root: PathBuf,
    pub reschedule_tx: mpsc::UnboundedSender<()>,
}

#[async_trait]
impl Tool for PlanDeferTool {
    fn name(&self) -> &'static str {
        "plan:defer"
    }

    fn description(&self) -> &'static str {
        "Schedule a wake alarm that resumes the on-disk working plan with a fresh tool budget. \
         Use when the mission must pause (tool-round budget, wait for a condition, or long gap) \
         and the agent should continue autonomously later — not for operator todos (agenda:*)."
    }

    fn allow_repeat_in_turn(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(PlanDeferArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: PlanDeferArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;

        let fire_at = match (args.minutes, args.hour, args.minute) {
            (Some(m), None, None) => {
                if m == 0 || m > MAX_TIMER_MINUTES {
                    return Err(FcpError::SchemaViolation(format!(
                        "minutes must be 1..={MAX_TIMER_MINUTES}"
                    )));
                }
                fire_at_from_secs(u64::from(m).saturating_mul(60))?
            }
            (None, Some(h), Some(mi)) => {
                let fire_dt = next_wall_alarm_fire_local(h, mi)?;
                fire_dt.timestamp() as u64
            }
            (None, None, None) => {
                return Err(FcpError::SchemaViolation(
                    "Provide either minutes (relative) or hour and minute (wall clock)."
                        .to_string(),
                ));
            }
            _ => {
                return Err(FcpError::SchemaViolation(
                    "Provide either minutes (relative) or hour+minute (wall), not both."
                        .to_string(),
                ));
            }
        };

        let note = args.note.as_deref().unwrap_or("");
        let scheduled =
            schedule_plan_resume(&self.workspace_root, fire_at, note, &self.reschedule_tx).await?;

        Ok(format!(
            "SUCCESS: Plan resume armed alarm [{}]; fire_at_unix={} label={:?}",
            scheduled.alarm_id, scheduled.fire_at_unix, scheduled.label
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::working_plan::{load, save, PlanStep, PlanStepStatus, WorkingPlan};
    use tempfile::tempdir;

    fn make_tool(dir: &std::path::Path) -> PlanDeferTool {
        let (tx, _rx) = mpsc::unbounded_channel();
        PlanDeferTool {
            workspace_root: dir.to_path_buf(),
            reschedule_tx: tx,
        }
    }

    async fn seed_open(dir: &std::path::Path) {
        save(
            dir,
            &WorkingPlan {
                goal: "g".into(),
                steps: vec![PlanStep {
                    id: "a".into(),
                    title: "Do thing".into(),
                    status: PlanStepStatus::Active,
                    kind: None,
                }],
                current_step_id: Some("a".into()),
                version: 1,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn defer_writes_plan_resume_alarm() -> Result<()> {
        let dir = tempdir().unwrap();
        seed_open(dir.path()).await;
        let tool = make_tool(dir.path());
        let result = tool
            .execute(serde_json::json!({ "minutes": 2, "note": "resume weather" }))
            .await?;
        assert!(result.contains("SUCCESS"), "result: {result}");
        let plan = load(dir.path()).await?.expect("plan");
        assert!(plan.resume_alarm_id.is_some());
        let alarms = crate::tools::clock::load_alarms(&crate::vault_layout::alarms_json(
            dir.path(),
        ))
        .await?;
        assert_eq!(alarms.len(), 1);
        assert_eq!(alarms[0].plan_resume, Some(true));
        assert_eq!(alarms[0].label, "resume weather");
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn defer_requires_open_plan() -> Result<()> {
        let dir = tempdir().unwrap();
        let tool = make_tool(dir.path());
        let err = tool
            .execute(serde_json::json!({ "minutes": 1 }))
            .await
            .expect_err("no plan");
        assert!(
            err.to_string().contains("No working plan") || err.to_string().contains("plan:set"),
            "err: {err}"
        );
        Ok(())
    }
}
