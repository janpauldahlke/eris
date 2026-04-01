use async_trait::async_trait;
use chrono::{Local, LocalResult, NaiveTime};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;

use crate::executive::error::{FcpError, Result};
use crate::tools::clock::{
    load_alarms, save_alarms, AlarmRecord, FCP_ALARMS_FILE, MAX_LABEL_CHARS,
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

fn next_fire_local(hour: u8, minute: u8) -> Result<chrono::DateTime<Local>> {
    if hour > 23 {
        return Err(FcpError::SchemaViolation("hour must be 0..=23".into()));
    }
    if minute > 59 {
        return Err(FcpError::SchemaViolation("minute must be 0..=59".into()));
    }
    let t = NaiveTime::from_hms_opt(u32::from(hour), u32::from(minute), 0)
        .ok_or_else(|| FcpError::SchemaViolation("invalid time".into()))?;
    let now = Local::now();
    let naive_day = now.date_naive().and_time(t);
    let dt = match naive_day.and_local_timezone(Local) {
        LocalResult::Single(d) => d,
        LocalResult::None => {
            return Err(FcpError::Config(
                "local time does not exist for this date (DST gap)".into(),
            ));
        }
        LocalResult::Ambiguous(earliest, _) => earliest,
    };
    if dt <= now {
        Ok(dt + chrono::Duration::days(1))
    } else {
        Ok(dt)
    }
}

#[async_trait]
impl Tool for ClockWallAlarmTool {
    fn name(&self) -> &'static str {
        "clock:alarm"
    }

    fn description(&self) -> &'static str {
        "Schedule a wall-clock alarm at hour:minute (24h local). If that time already passed today, schedules tomorrow."
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
        let fire_dt = next_fire_local(args.hour, args.minute)?;
        let fire_at = fire_dt.timestamp() as u64;

        let path = self.workspace_root.join(FCP_ALARMS_FILE);
        let mut alarms = load_alarms(&path).await?;
        let id = uuid::Uuid::new_v4().to_string();
        alarms.push(AlarmRecord {
            id,
            fire_at_unix: fire_at,
            label: args.label.clone(),
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
