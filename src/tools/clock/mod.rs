//! Clock and alarm tools: local time, relative timers, and wall-clock alarms.

mod now;
mod timer;
mod wall;

pub use now::{ClockNowTool, session_reference_time_block_for_prompt};
pub use timer::ClockTimerTool;
pub use wall::ClockWallAlarmTool;

use chrono::{Local, LocalResult, NaiveTime};
use serde::{Deserialize, Serialize};

use crate::executive::error::{FcpError, Result};
use tokio::fs;

pub const MAX_LABEL_CHARS: usize = 200;
pub const MAX_TIMER_MINUTES: u32 = 24 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlarmRecord {
    pub id: String,
    pub fire_at_unix: u64,
    pub label: String,
    /// When set, this alarm is tied to a row in `.fcp/tools/agenda.json` for confirmation/removal flows.
    #[serde(default)]
    pub agenda_task_id: Option<String>,
    /// Routes the fire payload at scheduler time: `Some("self")` triggers `AlarmPayload::AgendaSelfPrompt`
    /// (agent self-execution), anything else (or `None`) keeps the legacy user-confirm flow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agenda_kind: Option<String>,
}

/// Next local wall-clock fire time for hour:minute (24h). If that time already passed today, tomorrow.
pub(crate) fn next_wall_alarm_fire_local(hour: u8, minute: u8) -> Result<chrono::DateTime<Local>> {
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

/// Removes one alarm by `id`. Returns `Ok(true)` if a row was removed.
pub async fn remove_alarm_by_id(path: &std::path::Path, alarm_id: &str) -> Result<bool> {
    let mut alarms = load_alarms(path).await?;
    let initial = alarms.len();
    alarms.retain(|a| a.id != alarm_id);
    if alarms.len() == initial {
        return Ok(false);
    }
    save_alarms(path, &alarms).await?;
    Ok(true)
}

pub async fn load_alarms(path: &std::path::Path) -> Result<Vec<AlarmRecord>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path).await.map_err(FcpError::Io)?;
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&content).map_err(FcpError::ParseFault)
}

pub async fn save_alarms(path: &std::path::Path, alarms: &[AlarmRecord]) -> Result<()> {
    let data = serde_json::to_string_pretty(alarms).map_err(|e| FcpError::Config(e.to_string()))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await.map_err(FcpError::Io)?;
    }
    fs::write(path, data).await.map_err(FcpError::Io)?;
    Ok(())
}
