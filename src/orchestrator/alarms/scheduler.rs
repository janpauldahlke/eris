//! Background alarm scheduler: fires due rows from `.fcp/tools/alarms.json` and notifies the active
//! presentation channel via `try_send` only (never blocks the runtime).

use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::executive::error::Result;
use crate::presentation::{AlarmPayload, SessionEvent};
use crate::tools::agenda::{AgendaTask, AgendaTaskKind, SelfReminderPlan};
use crate::tools::clock::{AlarmRecord, load_alarms, save_alarms};

async fn load_self_plan_for_task(
    workspace_root: &std::path::Path,
    agenda_task_id: &str,
) -> Option<SelfReminderPlan> {
    let agenda_path = crate::vault_layout::agenda_json(workspace_root);
    let content = tokio::fs::read_to_string(&agenda_path).await.ok()?;
    if content.trim().is_empty() {
        return None;
    }
    let tasks: Vec<AgendaTask> = serde_json::from_str(&content).ok()?;
    let row = tasks.into_iter().find(|t| t.id == agenda_task_id)?;
    if row.kind != AgendaTaskKind::SelfDriven {
        return None;
    }
    let raw = row.plan?;
    serde_json::from_str(&raw).ok()
}

fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Computes the next sleep duration until the earliest future fire time, capped for sanity.
fn sleep_until_next(alarms: &[AlarmRecord]) -> Duration {
    let now = unix_now_secs();
    if alarms.iter().any(|a| a.fire_at_unix <= now) {
        return Duration::ZERO;
    }
    let next = alarms
        .iter()
        .map(|a| a.fire_at_unix)
        .filter(|&t| t > now)
        .min();
    match next {
        Some(t) => {
            let wait = t.saturating_sub(now);
            Duration::from_secs(wait.min(86400))
        }
        None => Duration::from_secs(60),
    }
}

async fn fire_due_and_persist(
    workspace_root: &std::path::Path,
    path: &std::path::Path,
    presentation_tx: &mpsc::Sender<SessionEvent>,
) -> Result<()> {
    let mut alarms = load_alarms(path).await?;
    let now = unix_now_secs();
    let mut remaining: Vec<AlarmRecord> = Vec::new();
    let mut due: Vec<AlarmRecord> = Vec::new();
    for a in alarms.drain(..) {
        if a.fire_at_unix <= now {
            due.push(a);
        } else {
            remaining.push(a);
        }
    }
    save_alarms(path, &remaining).await?;
    for a in due {
        let payload = if let Some(tid) = a.agenda_task_id.clone() {
            if a.agenda_kind.as_deref() == Some("self") {
                if let Some(plan) = load_self_plan_for_task(workspace_root, &tid).await {
                    AlarmPayload::AgendaSelfPrompt {
                        agenda_task_id: tid,
                        label: a.label,
                        plan: plan.hint,
                        checklist: plan.checklist,
                        alarm_record_id: a.id,
                        seconds_late: now.saturating_sub(a.fire_at_unix),
                    }
                } else {
                    AlarmPayload::AgendaLinked {
                        agenda_task_id: tid,
                        label: a.label,
                        alarm_record_id: a.id,
                        seconds_late: now.saturating_sub(a.fire_at_unix),
                    }
                }
            } else {
                AlarmPayload::AgendaLinked {
                    agenda_task_id: tid,
                    label: a.label,
                    alarm_record_id: a.id,
                    seconds_late: now.saturating_sub(a.fire_at_unix),
                }
            }
        } else {
            AlarmPayload::Plain(a.label)
        };
        if presentation_tx
            .try_send(SessionEvent::SystemAlarm(payload))
            .is_err()
        {
            tracing::error!("Dropped alarm due to presentation channel backpressure");
        }
    }
    Ok(())
}

pub fn spawn_alarm_scheduler(
    workspace_root: PathBuf,
    presentation_tx: mpsc::Sender<SessionEvent>,
    mut reschedule_rx: mpsc::UnboundedReceiver<()>,
    cancel_token: CancellationToken,
) {
    tokio::spawn(async move {
        let path = crate::vault_layout::alarms_json(&workspace_root);
        loop {
            let alarms = match load_alarms(&path).await {
                Ok(a) => a,
                Err(e) => {
                    tracing::warn!(error = %e, path = %path.display(), "alarm scheduler: failed to load alarms");
                    Vec::new()
                }
            };
            let sleep_for = sleep_until_next(&alarms);
            tokio::select! {
                biased;
                _ = tokio::time::sleep(sleep_for) => {
                    if let Err(e) = fire_due_and_persist(&workspace_root, &path, &presentation_tx).await {
                        tracing::warn!(error = %e, "alarm scheduler: fire/persist failed");
                    }
                }
                Some(()) = reschedule_rx.recv() => {
                    continue;
                }
                _ = cancel_token.cancelled() => {
                    break;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn fire_due_self_alarm_emits_agenda_self_prompt() {
        let dir = tempdir().expect("tmpdir");
        let tools_dir = crate::vault_layout::tools_dir(dir.path());
        tokio::fs::create_dir_all(&tools_dir)
            .await
            .expect("create tools dir");

        let agenda_path = crate::vault_layout::agenda_json(dir.path());
        tokio::fs::write(
            &agenda_path,
            r#"[{"id":"self1","created_at":1,"description":"loop task","status":"pending","alarm_id":"alarm1","kind":"self_driven","plan":"{\"hint\":\"continue the loop\",\"checklist\":[\"clock:now\",\"agenda:list\"]}"}]"#,
        )
        .await
        .expect("seed agenda");

        let now = unix_now_secs();
        let alarms_path = crate::vault_layout::alarms_json(dir.path());
        tokio::fs::write(
            &alarms_path,
            format!(
                r#"[{{"id":"alarm1","fire_at_unix":{},"label":"loop task","agenda_task_id":"self1","agenda_kind":"self"}}]"#,
                now.saturating_sub(1)
            ),
        )
        .await
        .expect("seed alarms");

        let (tx, mut rx) = mpsc::channel::<SessionEvent>(4);
        fire_due_and_persist(dir.path(), &alarms_path, &tx)
            .await
            .expect("fire due");

        let ev = rx.recv().await.expect("one session event");
        match ev {
            SessionEvent::SystemAlarm(AlarmPayload::AgendaSelfPrompt {
                agenda_task_id,
                label,
                plan,
                checklist,
                ..
            }) => {
                assert_eq!(agenda_task_id, "self1");
                assert_eq!(label, "loop task");
                assert_eq!(plan, "continue the loop");
                assert_eq!(checklist, vec!["clock:now".to_string(), "agenda:list".to_string()]);
            }
            other => panic!("unexpected alarm payload: {other:?}"),
        }

        let persisted = tokio::fs::read_to_string(&alarms_path)
            .await
            .expect("read alarms");
        assert_eq!(persisted.trim(), "[]");
    }
}
