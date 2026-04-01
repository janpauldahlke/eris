use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

use crate::executive::error::{FcpError, Result};
use crate::tools::clock::{
    load_alarms, save_alarms, AlarmRecord, FCP_ALARMS_FILE, MAX_LABEL_CHARS, MAX_TIMER_MINUTES,
};
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct ClockTimerArgs {
    pub minutes: u32,
    pub label: String,
}

pub struct ClockTimerTool {
    pub workspace_root: PathBuf,
    pub reschedule_tx: mpsc::UnboundedSender<()>,
}

#[async_trait]
impl Tool for ClockTimerTool {
    fn name(&self) -> &'static str {
        "clock:timer"
    }

    fn description(&self) -> &'static str {
        "Schedule a relative timer: fires after N minutes with the given label."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(ClockTimerArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: ClockTimerArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        if args.minutes == 0 || args.minutes > MAX_TIMER_MINUTES {
            return Err(FcpError::SchemaViolation(format!(
                "minutes must be 1..={MAX_TIMER_MINUTES}"
            )));
        }
        if args.label.is_empty() || args.label.len() > MAX_LABEL_CHARS {
            return Err(FcpError::SchemaViolation(format!(
                "label must be 1..={MAX_LABEL_CHARS} chars"
            )));
        }

        let path = self.workspace_root.join(FCP_ALARMS_FILE);
        let mut alarms = load_alarms(&path).await?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| FcpError::Config("system clock before UNIX epoch".into()))?;
        let fire_at = now.as_secs().saturating_add(u64::from(args.minutes).saturating_mul(60));
        let id = uuid::Uuid::new_v4().to_string();
        alarms.push(AlarmRecord {
            id,
            fire_at_unix: fire_at,
            label: args.label.clone(),
        });
        save_alarms(&path, &alarms).await?;
        let _ = self.reschedule_tx.send(());

        let when = chrono::DateTime::from_timestamp(fire_at as i64, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| fire_at.to_string());
        Ok(format!(
            "SUCCESS: Timer set for {} minutes; first fire at unix={} ({}) label={:?}",
            args.minutes, fire_at, when, args.label
        ))
    }
}
