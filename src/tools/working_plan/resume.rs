//! Schedule / cancel working-plan resume alarms (Phase 2A long-horizon wake).

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;

use super::{load, save, WorkingPlan};
use crate::executive::error::{FcpError, Result};
use crate::tools::clock::{load_alarms, remove_alarm_by_id, save_alarms, AlarmRecord};

/// Default label when the caller does not supply a note.
pub const DEFAULT_RESUME_LABEL: &str = "Continue working plan";

/// Result of arming a plan-resume alarm.
#[derive(Debug, Clone)]
pub struct PlanResumeSchedule {
    pub alarm_id: String,
    pub fire_at_unix: u64,
    pub label: String,
}

/// Cancel any prior `resume_alarm_id` on the plan (and the matching alarms.json row).
pub async fn cancel_plan_resume_alarm(workspace_root: &Path, plan: &mut WorkingPlan) -> Result<()> {
    if let Some(aid) = plan.resume_alarm_id.take() {
        let alarms_path = crate::vault_layout::alarms_json(workspace_root);
        let _ = remove_alarm_by_id(&alarms_path, &aid).await?;
    }
    Ok(())
}

/// Replace any existing plan-resume alarm, write a new `plan_resume` row, bump plan version.
pub async fn schedule_plan_resume(
    workspace_root: &Path,
    fire_at_unix: u64,
    label: &str,
    reschedule_tx: &mpsc::UnboundedSender<()>,
) -> Result<PlanResumeSchedule> {
    let mut plan = load(workspace_root).await?.ok_or_else(|| FcpError::ToolFault {
        tool_name: "plan:defer".into(),
        reason: "No working plan set; call plan:set before scheduling a resume.".into(),
    })?;
    if plan.open_steps().is_empty() {
        return Err(FcpError::ToolFault {
            tool_name: "plan:defer".into(),
            reason: "Working plan has no open steps; nothing to resume.".into(),
        });
    }

    cancel_plan_resume_alarm(workspace_root, &mut plan).await?;

    let label = {
        let t = label.trim();
        if t.is_empty() {
            DEFAULT_RESUME_LABEL.to_string()
        } else {
            t.chars().take(200).collect()
        }
    };

    let alarm_id = uuid::Uuid::new_v4().to_string();
    let alarms_path = crate::vault_layout::alarms_json(workspace_root);
    let mut alarms = load_alarms(&alarms_path).await?;
    alarms.push(AlarmRecord {
        id: alarm_id.clone(),
        fire_at_unix,
        label: label.clone(),
        agenda_task_id: None,
        agenda_kind: None,
        plan_resume: Some(true),
    });
    save_alarms(&alarms_path, &alarms).await?;

    plan.resume_alarm_id = Some(alarm_id.clone());
    plan.version = plan.version.saturating_add(1);
    plan.updated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    save(workspace_root, &plan).await?;

    let _ = reschedule_tx.send(());

    Ok(PlanResumeSchedule {
        alarm_id,
        fire_at_unix,
        label,
    })
}

/// Relative fire time: now + `secs` (minimum 1 second).
pub fn fire_at_from_secs(secs: u64) -> Result<u64> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| FcpError::Config("system clock before UNIX epoch".into()))?
        .as_secs();
    Ok(now.saturating_add(secs.max(1)))
}
