//! `presentation_rx` → `broadcast` fan-out and `SystemAlarm` → `UserAction` relay (parity with terminal).

use tokio::sync::{broadcast, mpsc};

use crate::presentation::{SessionEvent, UserAction, alarm_payload_to_user_action};

/// Forwards every [`SessionEvent`] to `events_tx` and mirrors [`SessionEvent::SystemAlarm`] onto `user_action_tx`.
pub fn spawn_presentation_bridge(
    mut presentation_rx: mpsc::Receiver<SessionEvent>,
    events_tx: broadcast::Sender<SessionEvent>,
    user_action_tx: mpsc::Sender<UserAction>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(evt) = presentation_rx.recv().await {
            if let SessionEvent::SystemAlarm(payload) = &evt {
                let action = alarm_payload_to_user_action(payload.clone());
                if user_action_tx.try_send(action).is_err() {
                    tracing::error!(
                        event = "fcp.web.bridge.alarm_dropped",
                        "Dropped alarm: user action channel full or session closed (presentation bridge)"
                    );
                } else {
                    tracing::debug!(
                        event = "fcp.web.bridge.alarm_relayed",
                        "Relayed SystemAlarm to user action channel"
                    );
                }
            }
            if let SessionEvent::ModelThought(ref t) = evt {
                tracing::debug!(
                    event = "fcp.web.bridge.model_thought",
                    thought_len = t.len(),
                    "Bridging ModelThought to SSE broadcast"
                );
            }
            if events_tx.send(evt).is_err() {
                tracing::debug!(
                    event = "fcp.web.bridge.broadcast_skipped",
                    "Session event not broadcast (no active SSE subscribers)"
                );
            }
        }
        tracing::debug!(
            event = "fcp.web.bridge.ended",
            "Presentation bridge task ended (channel closed)"
        );
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::{AlarmPayload, SessionEvent};

    #[tokio::test]
    async fn bridge_relays_system_alarm_to_user_action_and_broadcasts() {
        let (pres_tx, pres_rx) = mpsc::channel::<SessionEvent>(8);
        let (bc_tx, mut bc_sub) = broadcast::channel::<SessionEvent>(8);
        let (ua_tx, mut ua_rx) = mpsc::channel::<UserAction>(8);
        let _jh = spawn_presentation_bridge(pres_rx, bc_tx, ua_tx);

        pres_tx
            .send(SessionEvent::SystemAlarm(AlarmPayload::Plain(
                "wake".into(),
            )))
            .await
            .expect("send alarm");
        drop(pres_tx);

        let action = ua_rx.recv().await.expect("user action");
        assert!(matches!(
            action,
            UserAction::SystemInject(ref s) if s == "wake"
        ));

        let ev = bc_sub.recv().await.expect("broadcast");
        assert!(matches!(
            ev,
            SessionEvent::SystemAlarm(AlarmPayload::Plain(_))
        ));

        let _ = _jh.await;
    }

    #[tokio::test]
    async fn bridge_broadcasts_model_thought_for_sse() {
        let (pres_tx, pres_rx) = mpsc::channel::<SessionEvent>(8);
        let (bc_tx, mut bc_sub) = broadcast::channel::<SessionEvent>(8);
        let (ua_tx, _ua_rx) = mpsc::channel::<UserAction>(8);
        let _jh = spawn_presentation_bridge(pres_rx, bc_tx, ua_tx);

        pres_tx
            .send(SessionEvent::ModelThought("reasoning".into()))
            .await
            .expect("send thought");
        drop(pres_tx);

        let ev = bc_sub.recv().await.expect("broadcast");
        assert_eq!(ev, SessionEvent::ModelThought("reasoning".into()));

        let _ = _jh.await;
    }
}
