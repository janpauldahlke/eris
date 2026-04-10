//! Background alarm scheduler: fires due rows from `.fcp/tools/alarms.json` and notifies the active
//! presentation channel via `try_send` only (never blocks the runtime).

use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::executive::error::Result;
use crate::tools::clock::{load_alarms, save_alarms, AlarmRecord};
use crate::presentation::{AlarmPayload, SessionEvent};

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
            AlarmPayload::AgendaLinked {
                agenda_task_id: tid,
                label: a.label,
                alarm_record_id: a.id,
                seconds_late: now.saturating_sub(a.fire_at_unix),
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
                    if let Err(e) = fire_due_and_persist(&path, &presentation_tx).await {
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
