//! Startup hint when agenda-linked alarms are overdue (app was offline).

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::tools::clock::load_alarms;

/// If any agenda-linked alarm is already due, return a short banner for `TuiEvent::SystemError`.
pub async fn startup_overdue_agenda_hint(workspace_root: &Path) -> Option<String> {
    let path = crate::vault_layout::alarms_json(workspace_root);
    let alarms = load_alarms(&path).await.ok()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    let any = alarms
        .iter()
        .any(|a| a.fire_at_unix <= now && a.agenda_task_id.is_some());
    if any {
        Some(
            "[startup] One or more agenda-linked reminders were due while the app was not running; they will surface as alarms now."
                .into(),
        )
    } else {
        None
    }
}
