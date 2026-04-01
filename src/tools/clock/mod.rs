//! Clock and alarm tools: local time, relative timers, and wall-clock alarms.

mod now;
mod timer;
mod wall;

pub use now::ClockNowTool;
pub use timer::ClockTimerTool;
pub use wall::ClockWallAlarmTool;

use serde::{Deserialize, Serialize};

use crate::executive::error::{FcpError, Result};
use tokio::fs;

/// Workspace file storing pending alarms (JSON array).
pub const FCP_ALARMS_FILE: &str = ".fcp_alarms.json";
pub const MAX_LABEL_CHARS: usize = 200;
pub const MAX_TIMER_MINUTES: u32 = 24 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlarmRecord {
    pub id: String,
    pub fire_at_unix: u64,
    pub label: String,
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
    fs::write(path, data).await.map_err(FcpError::Io)?;
    Ok(())
}
