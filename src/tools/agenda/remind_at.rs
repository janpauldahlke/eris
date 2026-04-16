use async_trait::async_trait;
use schemars::schema::RootSchema;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs;
use tokio::sync::mpsc;

use crate::executive::error::{FcpError, Result};
use crate::tools::clock::{
    load_alarms, next_wall_alarm_fire_local, remove_alarm_by_id, save_alarms, AlarmRecord,
    MAX_TIMER_MINUTES,
};
use crate::tools::traits::Tool;
use super::AgendaTask;

#[derive(Deserialize, JsonSchema)]
pub struct AgendaRemindAtArgs {
    /// Existing agenda task id.
    pub task_id: Option<String>,
    /// New pending task description (creates a row like agenda:push).
    pub description: Option<String>,
    /// Relative reminder in N minutes.
    pub minutes: Option<u32>,
    /// Wall-clock hour (0–23) with `minute`.
    pub hour: Option<u8>,
    pub minute: Option<u8>,
}

pub struct AgendaRemindAtTool {
    pub workspace_root: PathBuf,
    pub reschedule_tx: mpsc::UnboundedSender<()>,
}

#[async_trait]
impl Tool for AgendaRemindAtTool {
    fn name(&self) -> &'static str {
        "agenda:remind_at"
    }

    fn description(&self) -> &'static str {
        "Agenda-linked reminder: writes/updates `.fcp/tools/agenda.json` and `.fcp/tools/alarms.json` (task_id or new description + relative minutes or wall-clock time). Not a generic clock/timer label."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        patch_agenda_remind_at_schema(schemars::schema_for!(AgendaRemindAtArgs))
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: AgendaRemindAtArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;

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
                    "Provide exactly one of task_id or description.".to_string(),
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
            (None, Some(h), Some(mi)) => Schedule::Wall { hour: h, minute: mi },
            (None, None, None) => {
                return Err(FcpError::SchemaViolation(
                    "Provide either minutes (relative) or hour and minute (wall clock).".to_string(),
                ));
            }
            _ => {
                return Err(FcpError::SchemaViolation(
                    "Provide either minutes (relative) or hour+minute (wall), not both.".to_string(),
                ));
            }
        };

        let agenda_path = crate::vault_layout::agenda_json(&self.workspace_root);
        let mut tasks: Vec<AgendaTask> = Vec::new();
        if agenda_path.exists() {
            let content = fs::read_to_string(&agenda_path).await.map_err(FcpError::Io)?;
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
            label = tasks[pos].description.clone();
            task_id = id;
        } else {
            let d = desc.ok_or_else(|| {
                FcpError::SchemaViolation("Provide exactly one of task_id or description.".to_string())
            })?;
            if d.len() > 200 {
                return Err(FcpError::SchemaViolation(
                    "description must be <= 200 chars".to_string(),
                ));
            }
            let normalized = d.trim();
            if let Some(pos) = tasks.iter().position(|t| {
                t.status == "pending" && t.description.trim().eq_ignore_ascii_case(normalized)
            }) {
                let old_alarm = tasks[pos].alarm_id.clone();
                if let Some(ref aid) = old_alarm {
                    let _ = remove_alarm_by_id(&alarms_path, aid).await?;
                }
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
                });
            }
        }

        let fire_at = match wall {
            Schedule::Minutes(m) => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_| FcpError::Config("system clock before UNIX epoch".into()))?;
                now.as_secs().saturating_add(u64::from(m).saturating_mul(60))
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
        });
        save_alarms(&alarms_path, &alarms).await?;

        let pos = tasks
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
        fs::write(&agenda_path, new_content).await.map_err(FcpError::Io)?;

        let _ = self.reschedule_tx.send(());

        Ok(format!(
            "SUCCESS: Agenda task [{}] linked to alarm [{}]; fire_at_unix={} label={:?}",
            task_id, alarm_uuid, fire_at, label
        ))
    }
}

enum Schedule {
    Minutes(u32),
    Wall { hour: u8, minute: u8 },
}

fn patch_agenda_remind_at_schema(base: RootSchema) -> RootSchema {
    // Keep the Rust-derived field schema, but enforce runtime XOR constraints directly in JSON Schema
    // so constrained decoding and Gatekeeper validation stay aligned.
    let Ok(mut value) = serde_json::to_value(&base) else {
        tracing::warn!("agenda:remind_at schema patch skipped: serialize failed");
        return base;
    };

    let Some(root_obj) = value.as_object_mut() else {
        tracing::warn!("agenda:remind_at schema patch skipped: root is not an object");
        return base;
    };

    let all_of = serde_json::json!([
        {
            "oneOf": [
                {
                    "required": ["task_id"],
                    "not": { "required": ["description"] }
                },
                {
                    "required": ["description"],
                    "not": { "required": ["task_id"] }
                }
            ]
        },
        {
            "oneOf": [
                {
                    "required": ["minutes"],
                    "not": {
                        "anyOf": [
                            { "required": ["hour"] },
                            { "required": ["minute"] }
                        ]
                    }
                },
                {
                    "required": ["hour", "minute"],
                    "not": { "required": ["minutes"] }
                }
            ]
        }
    ]);
    root_obj.insert("allOf".to_string(), all_of);

    match serde_json::from_value::<RootSchema>(value) {
        Ok(patched) => patched,
        Err(e) => {
            tracing::warn!(error = %e, "agenda:remind_at schema patch skipped: deserialize failed");
            base
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonschema::JSONSchema;
    use tempfile::tempdir;

    async fn write_agenda(dir: &std::path::Path, json: &str) -> Result<()> {
        let path = crate::vault_layout::agenda_json(dir);
        fs::create_dir_all(crate::vault_layout::tools_dir(dir))
            .await
            .map_err(FcpError::Io)?;
        fs::write(&path, json).await.map_err(FcpError::Io)?;
        Ok(())
    }

    #[tokio::test]
    async fn test_remind_at_new_description_minutes() -> Result<()> {
        let dir = tempdir().unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let tool = AgendaRemindAtTool {
            workspace_root: dir.path().to_path_buf(),
            reschedule_tx: tx,
        };
        let out = tool
            .execute(serde_json::json!({
                "description": "Buy milk",
                "minutes": 15
            }))
            .await?;
        assert!(out.contains("SUCCESS"));
        let agenda = fs::read_to_string(crate::vault_layout::agenda_json(dir.path()))
            .await
            .unwrap();
        let tasks: Vec<AgendaTask> = serde_json::from_str(&agenda).unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].alarm_id.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn test_remind_at_existing_task_replaces_alarm() -> Result<()> {
        let dir = tempdir().unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        write_agenda(
            dir.path(),
            r#"[{"id":"a1","created_at":1,"description":"x","status":"pending","alarm_id":"old-alarm"}]"#,
        )
        .await?;
        let alarms_path = crate::vault_layout::alarms_json(dir.path());
        fs::create_dir_all(crate::vault_layout::tools_dir(dir.path()))
            .await
            .unwrap();
        fs::write(
            &alarms_path,
            r#"[{"id":"old-alarm","fire_at_unix":9999999999,"label":"x","agenda_task_id":"a1"}]"#,
        )
        .await
        .unwrap();
        let tool = AgendaRemindAtTool {
            workspace_root: dir.path().to_path_buf(),
            reschedule_tx: tx,
        };
        let out = tool
            .execute(serde_json::json!({
                "task_id": "a1",
                "minutes": 5
            }))
            .await?;
        assert!(out.contains("SUCCESS"));
        let raw = fs::read_to_string(&alarms_path).await.unwrap();
        assert!(!raw.contains("old-alarm"));
        Ok(())
    }

    #[tokio::test]
    async fn test_remind_at_reuses_pending_same_description() -> Result<()> {
        let dir = tempdir().unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        write_agenda(
            dir.path(),
            r#"[{"id":"only1","created_at":1,"description":"Feed fish","status":"pending","alarm_id":null}]"#,
        )
        .await?;
        let tool = AgendaRemindAtTool {
            workspace_root: dir.path().to_path_buf(),
            reschedule_tx: tx,
        };
        let out = tool
            .execute(serde_json::json!({
                "description": "Feed fish",
                "minutes": 10
            }))
            .await?;
        assert!(out.contains("SUCCESS"));
        let agenda = fs::read_to_string(crate::vault_layout::agenda_json(dir.path()))
            .await
            .unwrap();
        let tasks: Vec<AgendaTask> = serde_json::from_str(&agenda).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "only1");
        assert!(tasks[0].alarm_id.is_some());
        Ok(())
    }

    #[test]
    fn test_parameters_schema_enforces_xor_and_schedule_mode() {
        let dir = tempdir().unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let tool = AgendaRemindAtTool {
            workspace_root: dir.path().to_path_buf(),
            reschedule_tx: tx,
        };
        let schema = serde_json::to_value(tool.parameters_schema()).expect("schema serializes");
        let compiled = JSONSchema::compile(&schema).expect("schema compiles");

        let valid_minutes = serde_json::json!({
            "description": "Call dentist",
            "minutes": 15
        });
        assert!(compiled.validate(&valid_minutes).is_ok());

        let invalid_target_both = serde_json::json!({
            "task_id": "a1",
            "description": "Call dentist",
            "minutes": 15
        });
        assert!(compiled.validate(&invalid_target_both).is_err());

        let invalid_schedule_both = serde_json::json!({
            "task_id": "a1",
            "minutes": 15,
            "hour": 9,
            "minute": 0
        });
        assert!(compiled.validate(&invalid_schedule_both).is_err());
    }
}
