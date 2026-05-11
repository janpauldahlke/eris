pub mod complete;
pub mod list;
pub mod push;
pub mod remind_at;
pub mod remind_self;
pub mod remove;

pub use complete::AgendaCompleteTool;
pub use list::AgendaListTool;
pub use push::AgendaPushTool;
pub use remind_at::AgendaRemindAtTool;
pub use remind_self::AgendaRemindSelfTool;
pub use remove::AgendaRemoveTool;

use serde::{Deserialize, Serialize};

/// Unique agenda row id (avoids second-granularity collisions from legacy hex ids).
pub fn new_task_id() -> String {
    format!("{:x}", uuid::Uuid::new_v4().as_u128())
}

/// Whether the row is a user-facing reminder (Done/Snooze framing) or a self-driven loop the agent
/// will execute when its alarm fires (SELF_REMINDER framing). Defaults to `User` so legacy
/// `agenda.json` files (no `kind` field) keep their current semantics.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgendaTaskKind {
    #[default]
    User,
    SelfDriven,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AgendaTask {
    pub id: String,
    pub created_at: u64,
    pub description: String,
    pub status: String,
    /// When set, points at a row in `.fcp/tools/alarms.json` for reminder scheduling.
    #[serde(default)]
    pub alarm_id: Option<String>,
    #[serde(default)]
    pub kind: AgendaTaskKind,
    /// JSON blob of `{ "hint": String, "checklist": Vec<String> }` for `kind = SelfDriven`.
    /// `None` for legacy / user-facing rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
}

/// Schema-stable shape for the agent self-reminder plan persisted in `AgendaTask::plan`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SelfReminderPlan {
    pub hint: String,
    #[serde(default)]
    pub checklist: Vec<String>,
}
