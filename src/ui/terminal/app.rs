use crate::config::LlmBackend;
use crate::engine::token_metrics::LlmTokenSnapshot;
use crate::executive::error::{FcpError, Result};
use crate::presentation::{
    AgentStateUpdate, AlarmPayload, InputSource, SessionEvent, UserAction, UserIngress,
};
use crossterm::event::{Event as CrosstermEvent, EventStream, KeyCode};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::Stdout;
use std::time::Duration;
use std::time::Instant;
use tokio::sync::{mpsc, watch};
use tokio_stream::StreamExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivePane {
    Main,
    Telemetry,
    SystemErrors,
    CommandDeck,
}

pub struct TuiApp {
    pub rx: mpsc::Receiver<SessionEvent>,
    pub action_tx: mpsc::Sender<UserAction>,
    pub input: String,
    pub chat_stack: Vec<String>,
    pub system_messages: Vec<String>,
    pub state: AgentStateUpdate,
    pub running: bool,
    pub chat_scroll: u16,
    pub chat_follow_latest: bool,
    pub telemetry_scroll: u16,
    pub system_errors_scroll: u16,
    pub command_deck_scroll: u16,
    pub command_deck_follow_latest: bool,
    pub active_pane: ActivePane,
    pub tick_count: u64,
    pub pending_inputs: usize,
    last_submit: Option<(String, Instant)>,
}

impl TuiApp {
    pub fn new(rx: mpsc::Receiver<SessionEvent>, action_tx: mpsc::Sender<UserAction>) -> Self {
        Self {
            rx,
            action_tx,
            input: String::new(),
            chat_stack: Vec::new(),
            system_messages: Vec::new(),
            state: AgentStateUpdate {
                state: crate::orchestrator::state::AgentState::Idle,
                tool_rounds: 0,
                max_tool_rounds: 5,
                recovery_count: 0,
                max_recovery_attempts: 3,
                active_task: None,
                activity_line: None,
                queued_inputs: 0,
                router_ms: 0,
                llm_ms: 0,
                tool_ms: 0,
                total_ms: 0,
                top_tool_match: None,
                llm_backend: LlmBackend::default(),
                llm_prompt_tokens: 0,
                llm_completion_tokens: 0,
                llm_last_generation_ms: 0,
                llm_last_tps_milli: 0,
                llm_tps_ewma_milli: 0,
            },
            running: true,
            chat_scroll: 0,
            chat_follow_latest: true,
            telemetry_scroll: 0,
            system_errors_scroll: 0,
            command_deck_scroll: 0,
            command_deck_follow_latest: true,
            active_pane: ActivePane::Main,
            tick_count: 0,
            pending_inputs: 0,
            last_submit: None,
        }
    }

    /// `token_metrics_rx` is optional; when present it is the UI-side clone of the watch receiver
    /// created with the engine in [`crate::executive::router`]. Metrics are engine-sourced, not owned here.
    pub async fn run(
        &mut self,
        mut terminal: Terminal<CrosstermBackend<Stdout>>,
        token_metrics_rx: Option<watch::Receiver<LlmTokenSnapshot>>,
    ) -> Result<()> {
        let mut reader = EventStream::new();
        let mut tick_interval = tokio::time::interval(Duration::from_millis(100));

        while self.running {
            tokio::select! {
                _ = tick_interval.tick() => {
                    self.tick_count = self.tick_count.wrapping_add(1);
                    let llm_tokens = token_metrics_rx
                        .as_ref()
                        .map(|rx| rx.borrow().clone())
                        .unwrap_or_default();
                    terminal.draw(|f| super::render::draw(f, self, &llm_tokens))
                        .map_err(|e| FcpError::Config(format!("Draw failed: {}", e)))?;
                }
                Some(Ok(evt)) = reader.next() => {
                    if let CrosstermEvent::Key(key) = evt {
                        // Hard exit
                        if key.code == KeyCode::Char('c') && key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) {
                            self.running = false;
                        } else {
                            // Input handling based on state
                            self.handle_input(key).await;
                        }
                    }
                }
                Some(evt) = self.rx.recv() => {
                    let mut redraw_now = false;
                    match evt {
                        SessionEvent::StateUpdate(update) => {
                            self.state = update;
                            redraw_now = true;
                        }
                        SessionEvent::ModelThought(t) => {
                            let n = t.chars().count();
                            let cap = 4000;
                            let body: String = t.chars().take(cap).collect();
                            let suffix = if n > cap { "…" } else { "" };
                            tracing::debug!(
                                event = "UI_RECV_MODEL_THOUGHT",
                                thought_len = n,
                                preview = %t.chars().take(100).collect::<String>(),
                                "Terminal: routing JSON `thought` to Telemetry pane"
                            );
                            self.system_messages
                                .push(format!("[model thought]{suffix}\n{body}"));
                            redraw_now = true;
                        }
                        SessionEvent::UserTranscriptLine { source, body, .. } => {
                            let badge = source.badge_label();
                            self.chat_stack.push(format!("[{badge}] {}", body));
                            self.chat_follow_latest = true;
                            redraw_now = true;
                        }
                        SessionEvent::IncomingMessage(msg) => {
                            let before_len = self.chat_stack.len();
                            self.chat_stack.push(msg);
                            // Only follow when the user was already following (at bottom); do not
                            // yank readers who scrolled up.
                            tracing::info!(
                                event = "UI_RECV_INCOMING_MESSAGE",
                                before_len,
                                after_len = self.chat_stack.len(),
                                follow_latest = self.chat_follow_latest,
                                chat_scroll = self.chat_scroll,
                                "Incoming message appended to deck"
                            );
                            self.pending_inputs = self.pending_inputs.saturating_sub(1);
                            redraw_now = true;
                        }
                        SessionEvent::SystemError(err) => {
                            self.system_messages.push(err);
                            redraw_now = true;
                        }
                        SessionEvent::SystemAlarm(payload) => {
                            let action = match payload {
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
                            };
                            if self.action_tx.try_send(action).is_err() {
                                tracing::error!("Dropped alarm due to presentation→orchestrator action channel backpressure");
                            }
                        }
                    }
                    if redraw_now {
                        let llm_tokens = token_metrics_rx
                            .as_ref()
                            .map(|rx| rx.borrow().clone())
                            .unwrap_or_default();
                        terminal
                            .draw(|f| super::render::draw(f, self, &llm_tokens))
                            .map_err(|e| FcpError::Config(format!("Draw failed: {}", e)))?;
                    }
                }
            }
        }
        Ok(())
    }

    async fn handle_input(&mut self, key: crossterm::event::KeyEvent) {
        // Locked input during Chat/Reflect/Recover. Unlocked during Idle (wakes) or WAIT_FOR_USER.
        // For structural initialization, we accept basic input.
        match key.code {
            KeyCode::Enter => {
                if !self.input.is_empty() {
                    let msg = self.input.clone();
                    self.input.clear();
                    self.command_deck_scroll = 0;
                    self.command_deck_follow_latest = true;

                    let trimmed = msg.trim();
                    if trimmed == "/exit" || trimmed == "/quit" {
                        self.running = false;
                        return;
                    }

                    self.chat_follow_latest = true;
                    let normalized = trimmed.to_lowercase();
                    let now = Instant::now();
                    let queued = self.state.queued_inputs.max(self.pending_inputs);
                    let busy = self.state.state != crate::orchestrator::state::AgentState::Idle
                        || queued > 0;
                    if busy {
                        if let Some((last_text, last_time)) = &self.last_submit
                            && *last_text == normalized
                            && now.duration_since(*last_time) <= Duration::from_secs(3)
                        {
                            self.system_messages
                                .push("[ui] Duplicate input suppressed while busy".to_string());
                            return;
                        }
                        if self.pending_inputs >= 3 {
                            self.system_messages.push("[ui] Queue full (3). Keeping latest, dropping oldest queued input.".to_string());
                            self.pending_inputs = 2;
                        }
                        self.system_messages.push(format!(
                            "[ui] Assistant busy. Message queued ({} pending).",
                            self.pending_inputs + 1
                        ));
                    }
                    self.last_submit = Some((normalized, now));
                    self.pending_inputs += 1;
                    let ingress = UserIngress {
                        source: InputSource::Cli,
                        display: trimmed.to_string(),
                        for_model: None,
                        image: None,
                        audio: None,
                    };
                    let _ = self
                        .action_tx
                        .send(UserAction::SubmitIngress(ingress))
                        .await;
                }
            }
            KeyCode::Esc => {
                let _ = self.action_tx.send(UserAction::CancelCurrentTurn).await;
                self.system_messages
                    .push("[ui] Cancel requested (Esc)".to_string());
            }
            KeyCode::Char(c) => {
                self.input.push(c);
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Tab => {
                self.active_pane = match self.active_pane {
                    ActivePane::Main => ActivePane::Telemetry,
                    ActivePane::Telemetry => ActivePane::SystemErrors,
                    ActivePane::SystemErrors => ActivePane::CommandDeck,
                    ActivePane::CommandDeck => ActivePane::Main,
                };
            }
            KeyCode::Up | KeyCode::PageUp => match self.active_pane {
                ActivePane::Main => {
                    self.chat_follow_latest = false;
                    self.chat_scroll = self.chat_scroll.saturating_sub(1);
                    tracing::debug!(
                        event = "UI_SCROLL_MAIN_UP",
                        follow_latest = self.chat_follow_latest,
                        chat_scroll = self.chat_scroll,
                        "Main deck scrolled up"
                    );
                }
                ActivePane::Telemetry => {
                    self.telemetry_scroll = self.telemetry_scroll.saturating_sub(1)
                }
                ActivePane::SystemErrors => {
                    self.system_errors_scroll = self.system_errors_scroll.saturating_sub(1)
                }
                ActivePane::CommandDeck => {
                    self.command_deck_follow_latest = false;
                    self.command_deck_scroll = self.command_deck_scroll.saturating_sub(1);
                }
            },
            KeyCode::Down | KeyCode::PageDown => match self.active_pane {
                ActivePane::Main => {
                    self.chat_follow_latest = false;
                    self.chat_scroll = self.chat_scroll.saturating_add(1);
                    tracing::debug!(
                        event = "UI_SCROLL_MAIN_DOWN",
                        follow_latest = self.chat_follow_latest,
                        chat_scroll = self.chat_scroll,
                        "Main deck scrolled down"
                    );
                }
                ActivePane::Telemetry => {
                    self.telemetry_scroll = self.telemetry_scroll.saturating_add(1)
                }
                ActivePane::SystemErrors => {
                    self.system_errors_scroll = self.system_errors_scroll.saturating_add(1)
                }
                ActivePane::CommandDeck => {
                    self.command_deck_follow_latest = false;
                    self.command_deck_scroll = self.command_deck_scroll.saturating_add(1);
                }
            },
            KeyCode::End => {
                match self.active_pane {
                    ActivePane::Main => self.chat_follow_latest = true,
                    ActivePane::CommandDeck => self.command_deck_follow_latest = true,
                    _ => {}
                }
                if self.active_pane == ActivePane::Main {
                    tracing::debug!(
                        event = "UI_SCROLL_MAIN_END",
                        follow_latest = self.chat_follow_latest,
                        chat_scroll = self.chat_scroll,
                        "Main deck follow-latest enabled"
                    );
                }
            }
            _ => {}
        }
    }
}
