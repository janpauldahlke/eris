//! When the Discord sidecar is enabled, a single task consumes `presentation_rx` and fans out to web broadcast, terminal, and a bounded Discord outbound queue (text only). [`SessionEvent::SystemAlarm`] is relayed to `user_action_tx` exactly once here.

use tokio::sync::{broadcast, mpsc};

use crate::presentation::alarm_relay::alarm_payload_to_user_action;
use crate::presentation::{SessionEvent, UserAction};

/// Targets for [`spawn_presentation_multiplex`].
pub struct PresentationMultiplexTargets {
    /// When present, every event is cloned to this broadcast (e.g. web SSE).
    pub web_broadcast: Option<broadcast::Sender<SessionEvent>>,
    /// When present, events are forwarded here except optionally [`SessionEvent::SystemAlarm`].
    pub terminal: Option<mpsc::Sender<SessionEvent>>,
    /// When true and `terminal` is set, [`SessionEvent::SystemAlarm`] is not sent to the terminal (relay happens only via `user_action_tx` in this task).
    pub terminal_omit_system_alarm: bool,
    pub user_action_tx: mpsc::Sender<UserAction>,
    /// Assistant lines only; mux uses [`mpsc::error::TrySendError`] — never blocks on Discord.
    pub discord_outbound: Option<mpsc::Sender<String>>,
}

/// Consumes the session presentation stream until the sender half is dropped.
pub fn spawn_presentation_multiplex(
    mut presentation_rx: mpsc::Receiver<SessionEvent>,
    targets: PresentationMultiplexTargets,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        tracing::info!(
            event = "fcp.presentation.mux.started",
            "Presentation multiplex started"
        );
        while let Some(evt) = presentation_rx.recv().await {
            if let SessionEvent::SystemAlarm(payload) = &evt {
                let action = alarm_payload_to_user_action(payload.clone());
                if targets.user_action_tx.try_send(action).is_err() {
                    tracing::error!(
                        event = "fcp.presentation.mux.alarm_dropped",
                        "Dropped alarm: user action channel full or session closed"
                    );
                } else {
                    tracing::debug!(
                        event = "fcp.presentation.mux.alarm_relayed",
                        "Relayed SystemAlarm to user action channel"
                    );
                }
            }
            if let Some(ref bc) = targets.web_broadcast {
                if bc.send(evt.clone()).is_err() {
                    tracing::debug!(
                        event = "fcp.presentation.mux.web_broadcast_skipped",
                        "Session event not broadcast (no active SSE subscribers)"
                    );
                }
            }
            if let Some(ref tui_tx) = targets.terminal {
                let forward_to_tui = !(targets.terminal_omit_system_alarm
                    && matches!(evt, SessionEvent::SystemAlarm(_)));
                if forward_to_tui && tui_tx.send(evt.clone()).await.is_err() {
                    tracing::warn!(
                        event = "fcp.presentation.mux.tui_forward_failed",
                        "TUI presentation channel closed"
                    );
                    break;
                }
            }
            if let SessionEvent::IncomingMessage(body) = &evt {
                if let Some(ref dtx) = targets.discord_outbound {
                    if dtx.try_send(body.clone()).is_err() {
                        tracing::warn!(
                            event = "fcp.presentation.mux.discord_queue_full",
                            message_len = body.len(),
                            "Discord outbound queue full or closed; dropped assistant line"
                        );
                    }
                }
            }
        }
        tracing::debug!(
            event = "fcp.presentation.mux.ended",
            "Presentation multiplex ended (rx closed)"
        );
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::AlarmPayload;

    #[tokio::test(flavor = "current_thread")]
    async fn multiplex_relays_alarm_once_and_forwards_incoming_to_discord_queue() {
        let (pres_tx, pres_rx) = mpsc::channel::<SessionEvent>(8);
        let (bc_tx, mut bc_sub) = broadcast::channel::<SessionEvent>(8);
        let (ua_tx, mut ua_rx) = mpsc::channel::<UserAction>(8);
        let (dtx, mut drx) = mpsc::channel::<String>(8);
        let _jh = spawn_presentation_multiplex(
            pres_rx,
            PresentationMultiplexTargets {
                web_broadcast: Some(bc_tx),
                terminal: None,
                terminal_omit_system_alarm: false,
                user_action_tx: ua_tx,
                discord_outbound: Some(dtx),
            },
        );

        pres_tx
            .send(SessionEvent::IncomingMessage("hello discord".into()))
            .await
            .expect("send");
        pres_tx
            .send(SessionEvent::SystemAlarm(AlarmPayload::Plain(
                "wake".into(),
            )))
            .await
            .expect("send alarm");
        drop(pres_tx);

        let line = drx.recv().await.expect("discord line");
        assert_eq!(line, "hello discord");

        let action = ua_rx.recv().await.expect("alarm action");
        assert!(matches!(action, UserAction::SystemInject(ref s) if s == "wake"));

        let ev = bc_sub.recv().await.expect("broadcast");
        assert!(matches!(ev, SessionEvent::IncomingMessage(_)));
        let ev2 = bc_sub.recv().await.expect("broadcast2");
        assert!(matches!(ev2, SessionEvent::SystemAlarm(_)));

        let _ = _jh.await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn multiplex_relays_self_prompt_alarm_to_user_action() {
        let (pres_tx, pres_rx) = mpsc::channel::<SessionEvent>(8);
        let (ua_tx, mut ua_rx) = mpsc::channel::<UserAction>(8);
        let _jh = spawn_presentation_multiplex(
            pres_rx,
            PresentationMultiplexTargets {
                web_broadcast: None,
                terminal: None,
                terminal_omit_system_alarm: false,
                user_action_tx: ua_tx,
                discord_outbound: None,
            },
        );

        pres_tx
            .send(SessionEvent::SystemAlarm(AlarmPayload::AgendaSelfPrompt {
                agenda_task_id: "t1".into(),
                label: "loop".into(),
                plan: "continue".into(),
                checklist: vec!["clock:now".into(), "agenda:list".into()],
                alarm_record_id: "a1".into(),
                seconds_late: 0,
            }))
            .await
            .expect("send self alarm");
        drop(pres_tx);

        let action = ua_rx.recv().await.expect("alarm action");
        assert!(matches!(
            action,
            UserAction::AgendaSelfPrompt {
                agenda_task_id,
                label,
                plan,
                checklist,
                alarm_record_id,
                seconds_late
            } if agenda_task_id == "t1"
                && label == "loop"
                && plan == "continue"
                && checklist == vec!["clock:now".to_string(), "agenda:list".to_string()]
                && alarm_record_id == "a1"
                && seconds_late == 0
        ));

        let _ = _jh.await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn user_transcript_line_broadcast_not_discord_queue() {
        use std::time::Duration;

        use crate::presentation::InputSource;

        let (pres_tx, pres_rx) = mpsc::channel::<SessionEvent>(8);
        let (bc_tx, mut bc_sub) = broadcast::channel::<SessionEvent>(8);
        let (ua_tx, _ua_rx) = mpsc::channel::<UserAction>(8);
        let (dtx, mut drx) = mpsc::channel::<String>(8);
        let _jh = spawn_presentation_multiplex(
            pres_rx,
            PresentationMultiplexTargets {
                web_broadcast: Some(bc_tx),
                terminal: None,
                terminal_omit_system_alarm: false,
                user_action_tx: ua_tx,
                discord_outbound: Some(dtx),
            },
        );

        pres_tx
            .send(SessionEvent::UserTranscriptLine {
                source: InputSource::Discord,
                body: "from discord".into(),
                image: None,
                audio: None,
            })
            .await
            .expect("send");
        drop(pres_tx);

        let ev = bc_sub.recv().await.expect("broadcast");
        assert!(matches!(
            ev,
            SessionEvent::UserTranscriptLine {
                source: InputSource::Discord,
                ..
            }
        ));

        let r = tokio::time::timeout(Duration::from_millis(80), drx.recv()).await;
        assert!(
            matches!(r, Err(_) | Ok(None)),
            "user transcript must not enqueue Discord outbound"
        );

        let _ = _jh.await;
    }
}
