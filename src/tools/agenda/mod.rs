pub mod push;
pub mod list;
pub mod complete;
pub mod remove;
pub mod remind_at;

pub use push::AgendaPushTool;
pub use list::AgendaListTool;
pub use complete::AgendaCompleteTool;
pub use remove::AgendaRemoveTool;
pub use remind_at::AgendaRemindAtTool;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct AgendaTask {
    pub id: String,
    pub created_at: u64,
    pub description: String,
    pub status: String,
    /// When set, points at a row in `.fcp_alarms.json` for reminder scheduling.
    #[serde(default)]
    pub alarm_id: Option<String>,
}
