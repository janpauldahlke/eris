//! Map [`AlarmPayload`](crate::presentation::AlarmPayload) to [`UserAction`](crate::presentation::UserAction) for scheduler → orchestrator relay.

use crate::presentation::{AlarmPayload, UserAction};

/// Same mapping as the web presentation bridge and the terminal alarm path.
pub fn alarm_payload_to_user_action(payload: AlarmPayload) -> UserAction {
    match payload {
        AlarmPayload::Plain(label) => UserAction::SystemInject(label),
        AlarmPayload::AgendaLinked {
            agenda_task_id,
            label,
            alarm_record_id,
            seconds_late,
        } => UserAction::AgendaAlarmPending {
            agenda_task_id,
            label,
            alarm_record_id,
            seconds_late,
        },
        AlarmPayload::AgendaSelfPrompt {
            agenda_task_id,
            label,
            plan,
            checklist,
            alarm_record_id,
            seconds_late,
        } => UserAction::AgendaSelfPrompt {
            agenda_task_id,
            label,
            plan,
            checklist,
            alarm_record_id,
            seconds_late,
        },
    }
}
