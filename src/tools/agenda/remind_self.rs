use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs;
use tokio::sync::mpsc;

use super::{AgendaTask, AgendaTaskKind, SelfReminderPlan};
use crate::executive::error::{FcpError, Result};
use crate::tools::clock::{
    AlarmRecord, MAX_TIMER_MINUTES, load_alarms, next_wall_alarm_fire_local, remove_alarm_by_id,
    save_alarms,
};
use crate::tools::traits::Tool;

const MAX_DESCRIPTION_CHARS: usize = 200;
const MAX_PLAN_CHARS: usize = 1500;
const MAX_CHECKLIST_ITEMS: usize = 8;
const MAX_CHECKLIST_ITEM_CHARS: usize = 120;

#[derive(Deserialize, JsonSchema)]
pub struct AgendaRemindSelfArgs {
    /// Existing agenda task id (extends a self-loop). XOR with `description`.
    pub task_id: Option<String>,
    /// New self-driven task description (short label, like agenda:push). XOR with `task_id`.
    pub description: Option<String>,
    /// Required: instructional payload the agent will read when the alarm fires; explain intent and rationale, not just a label.
    pub plan: String,
    /// Optional 1..=8 short checklist items the agent will work top-down.
    #[serde(default)]
    pub checklist: Option<Vec<String>>,
    /// Relative reminder in N minutes. XOR with `hour`+`minute`.
    pub minutes: Option<u32>,
    /// Wall-clock hour (0..=23) with `minute`. XOR with `minutes`.
    pub hour: Option<u8>,
    pub minute: Option<u8>,
}

pub struct AgendaRemindSelfTool {
    pub workspace_root: PathBuf,
    pub reschedule_tx: mpsc::UnboundedSender<()>,
}

#[async_trait]
impl Tool for AgendaRemindSelfTool {
    fn name(&self) -> &'static str {
        "agenda:remind_self"
    }

    fn description(&self) -> &'static str {
        "Self-driven agenda loop: schedules a future-self stimulus with a structured plan + optional checklist that the agent will execute autonomously when the alarm fires (no Done/Snooze prompt to the user). Use for multi-step background work the agent should resume on its own."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(AgendaRemindSelfArgs)
    }

    /// Same as `agenda:remind_at`: a self-loop typically reschedules itself with the same args repeatedly.
    fn allow_repeat_in_turn(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: AgendaRemindSelfArgs =
            serde_json::from_value(args).map_err(FcpError::ParseFault)?;

        let plan_text = args.plan.trim().to_string();
        if plan_text.is_empty() {
            return Err(FcpError::SchemaViolation(
                "plan must not be empty".to_string(),
            ));
        }
        if plan_text.len() > MAX_PLAN_CHARS {
            return Err(FcpError::SchemaViolation(format!(
                "plan must be <= {MAX_PLAN_CHARS} chars"
            )));
        }

        let checklist = match args.checklist {
            Some(items) => {
                if items.len() > MAX_CHECKLIST_ITEMS {
                    return Err(FcpError::SchemaViolation(format!(
                        "checklist must have at most {MAX_CHECKLIST_ITEMS} items"
                    )));
                }
                let mut cleaned: Vec<String> = Vec::with_capacity(items.len());
                for item in items {
                    let t = item.trim();
                    if t.is_empty() {
                        return Err(FcpError::SchemaViolation(
                            "checklist items must not be empty".to_string(),
                        ));
                    }
                    if t.len() > MAX_CHECKLIST_ITEM_CHARS {
                        return Err(FcpError::SchemaViolation(format!(
                            "checklist items must be <= {MAX_CHECKLIST_ITEM_CHARS} chars"
                        )));
                    }
                    cleaned.push(t.to_string());
                }
                cleaned
            }
            None => Vec::new(),
        };

        let tid = args
            .task_id
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let desc = args
            .description
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        match (&tid, &desc) {
            (Some(_), Some(_)) => {
                return Err(FcpError::SchemaViolation(
                    "Provide exactly one of task_id or description, not both.".to_string(),
                ));
            }
            (None, None) => {
                return Err(FcpError::SchemaViolation(
                    "Provide exactly one of task_id or description.".to_string(),
                ));
            }
            _ => {}
        }

        let wall = match (args.minutes, args.hour, args.minute) {
            (Some(m), None, None) => {
                if m == 0 || m > MAX_TIMER_MINUTES {
                    return Err(FcpError::SchemaViolation(format!(
                        "minutes must be 1..={MAX_TIMER_MINUTES}"
                    )));
                }
                Schedule::Minutes(m)
            }
            (None, Some(h), Some(mi)) => Schedule::Wall {
                hour: h,
                minute: mi,
            },
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

        let plan_blob = serde_json::to_string(&SelfReminderPlan {
            hint: plan_text.clone(),
            checklist: checklist.clone(),
        })
        .map_err(|e| FcpError::Config(e.to_string()))?;

        let agenda_path = crate::vault_layout::agenda_json(&self.workspace_root);
        let mut tasks: Vec<AgendaTask> = Vec::new();
        if agenda_path.exists() {
            let content = fs::read_to_string(&agenda_path)
                .await
                .map_err(FcpError::Io)?;
            if !content.trim().is_empty() {
                tasks = serde_json::from_str(&content).map_err(FcpError::ParseFault)?;
            }
        }

        let alarms_path = crate::vault_layout::alarms_json(&self.workspace_root);
        let task_id: String;
        let label: String;

        if let Some(id) = tid {
            let pos = tasks
                .iter()
                .position(|t| t.id == id)
                .ok_or_else(|| FcpError::ToolFault {
                    tool_name: self.name().into(),
                    reason: format!("Task ID {} not found", id),
                })?;
            let old_alarm = tasks[pos].alarm_id.clone();
            if let Some(ref aid) = old_alarm {
                let _ = remove_alarm_by_id(&alarms_path, aid).await?;
            }
            tasks[pos].kind = AgendaTaskKind::SelfDriven;
            tasks[pos].plan = Some(plan_blob.clone());
            label = tasks[pos].description.clone();
            task_id = id;
        } else {
            let d = desc.ok_or_else(|| {
                FcpError::SchemaViolation(
                    "Provide exactly one of task_id or description.".to_string(),
                )
            })?;
            if d.len() > MAX_DESCRIPTION_CHARS {
                return Err(FcpError::SchemaViolation(format!(
                    "description must be <= {MAX_DESCRIPTION_CHARS} chars"
                )));
            }
            let normalized = d.trim();
            if let Some(pos) = tasks.iter().position(|t| {
                t.status == "pending"
                    && t.kind == AgendaTaskKind::SelfDriven
                    && t.description.trim().eq_ignore_ascii_case(normalized)
            }) {
                let old_alarm = tasks[pos].alarm_id.clone();
                if let Some(ref aid) = old_alarm {
                    let _ = remove_alarm_by_id(&alarms_path, aid).await?;
                }
                tasks[pos].plan = Some(plan_blob.clone());
                task_id = tasks[pos].id.clone();
                label = tasks[pos].description.clone();
            } else {
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_| FcpError::Config("system clock before UNIX epoch".into()))?
                    .as_secs();
                task_id = super::new_task_id();
                label = d.clone();
                tasks.push(AgendaTask {
                    id: task_id.clone(),
                    created_at: timestamp,
                    description: d,
                    status: "pending".to_string(),
                    alarm_id: None,
                    kind: AgendaTaskKind::SelfDriven,
                    plan: Some(plan_blob.clone()),
                });
            }
        }

        let fire_at = match wall {
            Schedule::Minutes(m) => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_| FcpError::Config("system clock before UNIX epoch".into()))?;
                now.as_secs()
                    .saturating_add(u64::from(m).saturating_mul(60))
            }
            Schedule::Wall { hour, minute } => {
                let fire_dt = next_wall_alarm_fire_local(hour, minute)?;
                fire_dt.timestamp() as u64
            }
        };

        let alarm_uuid = uuid::Uuid::new_v4().to_string();
        let mut alarms = load_alarms(&alarms_path).await?;
        alarms.push(AlarmRecord {
            id: alarm_uuid.clone(),
            fire_at_unix: fire_at,
            label: label.clone(),
            agenda_task_id: Some(task_id.clone()),
            agenda_kind: Some("self".to_string()),
        });
        save_alarms(&alarms_path, &alarms).await?;

        let pos =
            tasks
                .iter()
                .position(|t| t.id == task_id)
                .ok_or_else(|| FcpError::ToolFault {
                    tool_name: self.name().into(),
                    reason: "Agenda task row missing after schedule".into(),
                })?;
        tasks[pos].alarm_id = Some(alarm_uuid.clone());

        let new_content =
            serde_json::to_string_pretty(&tasks).map_err(|e| FcpError::Config(e.to_string()))?;
        fs::create_dir_all(crate::vault_layout::tools_dir(&self.workspace_root))
            .await
            .map_err(FcpError::Io)?;
        fs::write(&agenda_path, new_content)
            .await
            .map_err(FcpError::Io)?;

        let _ = self.reschedule_tx.send(());

        Ok(format!(
            "SUCCESS: Self-driven agenda task [{}] linked to alarm [{}]; fire_at_unix={} label={:?} checklist_steps={}",
            task_id,
            alarm_uuid,
            fire_at,
            label,
            checklist.len()
        ))
    }
}

enum Schedule {
    Minutes(u32),
    Wall { hour: u8, minute: u8 },
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_tool(dir: &std::path::Path) -> AgendaRemindSelfTool {
        let (tx, _rx) = mpsc::unbounded_channel();
        AgendaRemindSelfTool {
            workspace_root: dir.to_path_buf(),
            reschedule_tx: tx,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn new_description_writes_self_kind_and_plan_blob() -> Result<()> {
        let dir = tempdir().expect("tmp");
        let tool = make_tool(dir.path());
        let out = tool
            .execute(serde_json::json!({
                "description": "Browse Moltbook again",
                "plan": "Open home, scan top 3 threads, post one welcome.",
                "checklist": ["clock:now", "moltbook:home", "moltbook:comment"],
                "minutes": 5,
            }))
            .await?;
        assert!(out.contains("SUCCESS"));
        let agenda = fs::read_to_string(crate::vault_layout::agenda_json(dir.path()))
            .await
            .expect("read agenda.json");
        let tasks: Vec<AgendaTask> = serde_json::from_str(&agenda).expect("parse tasks");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].kind, AgendaTaskKind::SelfDriven);
        let plan_raw = tasks[0].plan.as_ref().expect("plan stored");
        let plan: SelfReminderPlan =
            serde_json::from_str(plan_raw).expect("plan blob is valid SelfReminderPlan json");
        assert!(plan.hint.contains("Open home"));
        assert_eq!(plan.checklist.len(), 3);
        assert!(tasks[0].alarm_id.is_some());

        let alarms_raw = fs::read_to_string(crate::vault_layout::alarms_json(dir.path()))
            .await
            .expect("read alarms.json");
        let alarms: Vec<AlarmRecord> =
            serde_json::from_str(&alarms_raw).expect("parse alarms");
        assert_eq!(alarms.len(), 1);
        assert_eq!(alarms[0].agenda_kind.as_deref(), Some("self"));
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn missing_plan_rejected() {
        let dir = tempdir().expect("tmp");
        let tool = make_tool(dir.path());
        let err = tool
            .execute(serde_json::json!({
                "description": "x",
                "plan": "   ",
                "minutes": 5,
            }))
            .await
            .expect_err("schema");
        assert!(matches!(err, FcpError::SchemaViolation(_)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn checklist_cap_enforced() {
        let dir = tempdir().expect("tmp");
        let tool = make_tool(dir.path());
        let too_many: Vec<String> = (0..(MAX_CHECKLIST_ITEMS + 1))
            .map(|i| format!("step {i}"))
            .collect();
        let err = tool
            .execute(serde_json::json!({
                "description": "x",
                "plan": "ok",
                "checklist": too_many,
                "minutes": 5,
            }))
            .await
            .expect_err("schema");
        assert!(matches!(err, FcpError::SchemaViolation(_)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn existing_task_id_replaces_alarm_and_promotes_to_self() -> Result<()> {
        let dir = tempdir().expect("tmp");
        let agenda_path = crate::vault_layout::agenda_json(dir.path());
        fs::create_dir_all(crate::vault_layout::tools_dir(dir.path()))
            .await
            .expect("mkdir");
        fs::write(
            &agenda_path,
            r#"[{"id":"a1","created_at":1,"description":"continue research","status":"pending","alarm_id":"old-alarm"}]"#,
        )
        .await
        .expect("seed agenda");
        let alarms_path = crate::vault_layout::alarms_json(dir.path());
        fs::write(
            &alarms_path,
            r#"[{"id":"old-alarm","fire_at_unix":9999999999,"label":"continue research","agenda_task_id":"a1"}]"#,
        )
        .await
        .expect("seed alarm");

        let tool = make_tool(dir.path());
        let out = tool
            .execute(serde_json::json!({
                "task_id": "a1",
                "plan": "Resume research: re-read the last note, then summarize.",
                "checklist": ["vault:read", "memory:stage"],
                "minutes": 10,
            }))
            .await?;
        assert!(out.contains("SUCCESS"));

        let alarms_raw = fs::read_to_string(&alarms_path).await.expect("alarms");
        assert!(!alarms_raw.contains("old-alarm"));

        let agenda_raw = fs::read_to_string(&agenda_path).await.expect("agenda");
        let tasks: Vec<AgendaTask> = serde_json::from_str(&agenda_raw).expect("parse");
        assert_eq!(tasks[0].kind, AgendaTaskKind::SelfDriven);
        assert!(tasks[0].plan.is_some());
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn legacy_agenda_row_parses_with_default_kind() {
        let raw = r#"[{"id":"a1","created_at":1,"description":"old","status":"pending"}]"#;
        let tasks: Vec<AgendaTask> = serde_json::from_str(raw).expect("parse legacy");
        assert_eq!(tasks[0].kind, AgendaTaskKind::User);
        assert!(tasks[0].plan.is_none());
        assert!(tasks[0].alarm_id.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn neither_task_nor_description_rejected() {
        let dir = tempdir().expect("tmp");
        let tool = make_tool(dir.path());
        let err = tool
            .execute(serde_json::json!({
                "plan": "go",
                "minutes": 5,
            }))
            .await
            .expect_err("schema");
        assert!(matches!(err, FcpError::SchemaViolation(_)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn both_schedules_rejected() {
        let dir = tempdir().expect("tmp");
        let tool = make_tool(dir.path());
        let err = tool
            .execute(serde_json::json!({
                "description": "x",
                "plan": "y",
                "minutes": 5,
                "hour": 9,
                "minute": 0,
            }))
            .await
            .expect_err("schema");
        assert!(matches!(err, FcpError::SchemaViolation(_)));
    }
}
