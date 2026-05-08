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

/// Prefix applied by the orchestrator when turning [`UserAction::AgendaSelfPrompt`] into a `user` line.
/// The agent reads this as instruction to autonomously execute the stored plan + checklist (no Done/Snooze prompt).
pub const SYSTEM_SELF_REMINDER_PREFIX: &str = "[SYSTEM OVERRIDE - SELF REMINDER]: ";

/// Alarm notification from the scheduler: plain timer/wall, agenda-linked (user Done/Snooze flow),
/// or agent self-driven (the SELF_REMINDER protocol — agent executes plan + checklist autonomously).
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
    AgendaSelfPrompt {
        agenda_task_id: String,
        label: String,
        /// Free-text instructional payload the agent stored when scheduling.
        plan: String,
        /// Optional ordered steps; rendered to the agent as `- [ ] step` lines.
        checklist: Vec<String>,
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
    /// Agent self-driven alarm: orchestrator injects SELF_REMINDER framing with the stored plan
    /// + checklist; agent executes autonomously and calls `agenda:complete` (or extends with
    /// `agenda:remind_self`) on its own — no Done/Snooze prompt to the user.
    AgendaSelfPrompt {
        agenda_task_id: String,
        label: String,
        plan: String,
        checklist: Vec<String>,
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
            UserAction::AgendaSelfPrompt {
                agenda_task_id: "t2".into(),
                label: "browse".into(),
                plan: "open the home and skim".into(),
                checklist: vec!["clock:now".into(), "moltbook:home".into()],
                alarm_record_id: "a2".into(),
                seconds_late: 3,
            },
        ];
        for a in cases {
            let j = serde_json::to_string(&a).expect("serialize UserAction");
            let back: UserAction = serde_json::from_str(&j).expect("deserialize UserAction");
            assert_eq!(a, back);
        }
    }

    #[test]
    fn alarm_payload_json_roundtrip_self_prompt() {
        let payload = AlarmPayload::AgendaSelfPrompt {
            agenda_task_id: "t".into(),
            label: "L".into(),
            plan: "P".into(),
            checklist: vec!["a".into(), "b".into()],
            alarm_record_id: "ar".into(),
            seconds_late: 0,
        };
        let j = serde_json::to_string(&payload).expect("serialize AlarmPayload");
        let back: AlarmPayload = serde_json::from_str(&j).expect("deserialize AlarmPayload");
        assert_eq!(payload, back);
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
