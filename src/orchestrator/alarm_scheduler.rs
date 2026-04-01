//! Background alarm scheduler: fires due rows from `.fcp_alarms.json` and notifies the TUI via
//! `try_send` only (never blocks the runtime).

use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::executive::error::Result;
use crate::tools::clock::{load_alarms, save_alarms, AlarmRecord};
use crate::ui::events::TuiEvent;

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
    tui_tx: &mpsc::Sender<TuiEvent>,
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
        let label = a.label;
        if tui_tx
            .try_send(TuiEvent::SystemAlarm(label))
            .is_err()
        {
            tracing::error!("Dropped alarm due to TUI backpressure");
        }
    }
    Ok(())
}

pub fn spawn_alarm_scheduler(
    workspace_root: PathBuf,
    tui_tx: mpsc::Sender<TuiEvent>,
    mut reschedule_rx: mpsc::UnboundedReceiver<()>,
    cancel_token: CancellationToken,
) {
    tokio::spawn(async move {
        let path = workspace_root.join(crate::tools::clock::FCP_ALARMS_FILE);
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
                    if let Err(e) = fire_due_and_persist(&path, &tui_tx).await {
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
