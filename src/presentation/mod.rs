//! Presentation-neutral types shared by CLI (Ratatui) and web UI.
//! Interactive chat requires a live `presentation_tx`; `None` is for headless tests and batch runners.

pub mod alarm_relay;
pub mod multiplex;

pub use alarm_relay::alarm_payload_to_user_action;

use serde::{Deserialize, Serialize};

use crate::orchestrator::state::AgentState;

/// Where a user line entered the session (for transcript badges in web and TUI).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputSource {
    Web,
    Cli,
    Discord,
}

impl InputSource {
    /// Short label for badges (lowercase, no spaces).
    #[must_use]
    pub fn badge_label(self) -> &'static str {
        match self {
            InputSource::Web => "web",
            InputSource::Cli => "cli",
            InputSource::Discord => "discord",
        }
    }
}

/// One queued user turn: what UIs show (`display`) vs what the model receives (`for_model`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserIngress {
    pub source: InputSource,
    /// Plain text for transcript badges (e.g. Discord message body without framing).
    pub display: String,
    /// When set, pushed to the LLM stack as `user` content; otherwise `display` is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub for_model: Option<String>,
}

/// Prefix applied by the orchestrator when turning [`UserAction::SystemInject`] into a `user` line.
pub const SYSTEM_ALARM_PREFIX: &str = "[SYSTEM OVERRIDE - ALARM TRIGGERED]: ";

/// Alarm notification from the scheduler: plain timer/wall, or agenda-linked (needs confirmation flow).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlarmPayload {
    Plain(String),
    AgendaLinked {
        agenda_task_id: String,
        label: String,
        alarm_record_id: String,
        /// Seconds after scheduled fire (e.g. app was offline).
        seconds_late: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UserAction {
    Submit(String),
    /// Preferred path: includes [`InputSource`] for transcript badges; optional `for_model` for LLM-only framing.
    SubmitIngress(UserIngress),
    CancelCurrentTurn,
    /// Asynchronous clock/alarm injected via the active view; raw label only (prefix added in orchestrator).
    SystemInject(String),
    /// Agenda-linked alarm: orchestrator injects confirmation framing (same turn as live alarm).
    AgendaAlarmPending {
        agenda_task_id: String,
        label: String,
        alarm_record_id: String,
        seconds_late: u64,
    },
}

/// Outbound updates from core to the active presentation (terminal or web).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionEvent {
    StateUpdate(AgentStateUpdate),
    /// User line accepted into the session queue (before the model runs); drives web/TUI transcript with source badge.
    UserTranscriptLine {
        source: InputSource,
        body: String,
    },
    IncomingMessage(String),
    /// JSON protocol `thought` string from the assistant reply (internal reasoning; not `message_to_user`).
    ModelThought(String),
    SystemError(String),
    /// Fired by the alarm scheduler; the active view forwards to [`UserAction`] (plain inject or agenda confirmation).
    SystemAlarm(AlarmPayload),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentStateUpdate {
    pub state: AgentState,
    pub tool_rounds: u8,
    /// Configured per-turn cap (shown in Status as `T:…/max`).
    pub max_tool_rounds: u8,
    pub recovery_count: u8,
    /// Configured recovery budget (shown in Status as `R:…/max`).
    pub max_recovery_attempts: u8,
    pub active_task: Option<String>,
    /// Status-only hint while tools run (e.g. `Tools: …`); user text stays on the main transcript.
    pub activity_line: Option<String>,
    pub queued_inputs: usize,
    pub router_ms: u64,
    pub llm_ms: u64,
    pub tool_ms: u64,
    pub total_ms: u64,
    pub top_tool_match: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_action_json_roundtrip() {
        let cases = [
            UserAction::Submit("hello".into()),
            UserAction::SubmitIngress(UserIngress {
                source: InputSource::Discord,
                display: "ping".into(),
                for_model: Some("[Discord] ping".into()),
            }),
            UserAction::CancelCurrentTurn,
            UserAction::SystemInject("water".into()),
            UserAction::AgendaAlarmPending {
                agenda_task_id: "t1".into(),
                label: "x".into(),
                alarm_record_id: "a1".into(),
                seconds_late: 0,
            },
        ];
        for a in cases {
            let j = serde_json::to_string(&a).expect("serialize UserAction");
            let back: UserAction = serde_json::from_str(&j).expect("deserialize UserAction");
            assert_eq!(a, back);
        }
    }

    #[test]
    fn session_event_json_roundtrip_model_thought() {
        let ev = SessionEvent::ModelThought("step by step".into());
        let j = serde_json::to_string(&ev).expect("serialize");
        let back: SessionEvent = serde_json::from_str(&j).expect("deserialize");
        assert_eq!(ev, back);
    }

    #[test]
    fn session_event_json_roundtrip_user_transcript_line() {
        let ev = SessionEvent::UserTranscriptLine {
            source: InputSource::Discord,
            body: "hello from #nemos-home".into(),
        };
        let j = serde_json::to_string(&ev).expect("serialize");
        let back: SessionEvent = serde_json::from_str(&j).expect("deserialize");
        assert_eq!(ev, back);
    }
}
