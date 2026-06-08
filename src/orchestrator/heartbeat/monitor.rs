use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio_util::sync::CancellationToken;

pub fn spawn_heartbeat_monitor(
    last_input_time: Arc<AtomicU64>,
    idle_timeout_secs: u64,
    idle_trigger_tx: tokio::sync::watch::Sender<()>,
    cancel_token: CancellationToken,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                    let last = last_input_time.load(Ordering::Relaxed);
                    if now.saturating_sub(last) > idle_timeout_secs {
                        let _ = idle_trigger_tx.send(());
                        break;
                    }
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
    use std::time::Duration;

    #[tokio::test(flavor = "current_thread")]
    async fn test_heartbeat_triggers_idle_signal() {
        let last_input_time = Arc::new(AtomicU64::new(0)); // 0 represents way in the past
        let (tx, mut rx) = tokio::sync::watch::channel(());
        let cancel_token = CancellationToken::new();

        spawn_heartbeat_monitor(
            last_input_time.clone(),
            0, // Immediately trigger
            tx,
            cancel_token.clone(),
        );

        // It should fire almost immediately
        let res = tokio::time::timeout(Duration::from_secs(2), rx.changed()).await;
        assert!(res.is_ok(), "Heartbeat did not trigger idle signal");
    }
}
