use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;

use crate::executive::error::{FcpError, Result};
use crate::tools::clock::{
    load_alarms, next_wall_alarm_fire_local, save_alarms, AlarmRecord, MAX_LABEL_CHARS,
};
use crate::tools::traits::Tool;
use tokio::sync::mpsc;

#[derive(Deserialize, JsonSchema)]
pub struct ClockWallArgs {
    pub hour: u8,
    pub minute: u8,
    pub label: String,
}

pub struct ClockWallAlarmTool {
    pub workspace_root: PathBuf,
    pub reschedule_tx: mpsc::UnboundedSender<()>,
}

#[async_trait]
impl Tool for ClockWallAlarmTool {
    fn name(&self) -> &'static str {
        "clock:alarm"
    }

    fn description(&self) -> &'static str {
        "Standalone wall-clock alarm (hour:minute local + label); not tied to an agenda task. Use agenda:remind_at to link a reminder to a queued agenda item."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(ClockWallArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: ClockWallArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        if args.label.is_empty() || args.label.len() > MAX_LABEL_CHARS {
            return Err(FcpError::SchemaViolation(format!(
                "label must be 1..={MAX_LABEL_CHARS} chars"
            )));
        }
        let fire_dt = next_wall_alarm_fire_local(args.hour, args.minute)?;
        let fire_at = fire_dt.timestamp() as u64;

        let path = crate::vault_layout::alarms_json(&self.workspace_root);
        let mut alarms = load_alarms(&path).await?;
        let id = uuid::Uuid::new_v4().to_string();
        alarms.push(AlarmRecord {
            id,
            fire_at_unix: fire_at,
            label: args.label.clone(),
            agenda_task_id: None,
        });
        save_alarms(&path, &alarms).await?;
        let _ = self.reschedule_tx.send(());

        Ok(format!(
            "SUCCESS: Alarm locked for {} (local), unix={} label={:?}",
            fire_dt.format("%Y-%m-%d %H:%M:%S %Z"),
            fire_at,
            args.label
        ))
    }
}
