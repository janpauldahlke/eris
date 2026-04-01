use tokio::sync::mpsc;
use crate::executive::error::{Result, FcpError};
use crate::ui::events::{TuiEvent, AgentStateUpdate};
use crossterm::event::{Event as CrosstermEvent, EventStream, KeyCode};
use tokio_stream::StreamExt;
use std::time::Duration;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::Stdout;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivePane {
    Main,
    Telemetry,
    SystemErrors,
    CommandDeck,
}

pub struct TuiApp {
    pub rx: mpsc::Receiver<TuiEvent>,
    pub action_tx: mpsc::Sender<String>,
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
}

impl TuiApp {
    pub fn new(rx: mpsc::Receiver<TuiEvent>, action_tx: mpsc::Sender<String>) -> Self {
        Self {
            rx,
            action_tx,
            input: String::new(),
            chat_stack: Vec::new(),
            system_messages: Vec::new(),
            state: AgentStateUpdate {
                state: crate::orchestrator::state::AgentState::Idle,
                tool_rounds: 0,
                recovery_count: 0,
                active_task: None,
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
        }
    }

    pub async fn run(&mut self, mut terminal: Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
        let mut reader = EventStream::new();
        let mut tick_interval = tokio::time::interval(Duration::from_millis(100));

        while self.running {
            tokio::select! {
                _ = tick_interval.tick() => {
                    self.tick_count = self.tick_count.wrapping_add(1);
                    terminal.draw(|f| crate::ui::render::draw(f, self))
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
                    match evt {
                        TuiEvent::StateUpdate(update) => self.state = update,
                        TuiEvent::IncomingMessage(msg) => {
                            self.chat_stack.push(msg);
                            if self.chat_follow_latest {
                                self.chat_scroll = u16::MAX;
                            }
                        }
                        TuiEvent::SystemError(err) => self.system_messages.push(err),
                        _ => {}
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

                    self.chat_stack.push(format!("You: {}", trimmed));
                    let _ = self.action_tx.send(msg).await;
                }
            }
            KeyCode::Char(c) => {
                self.input.push(c);
                self.command_deck_follow_latest = true;
            }
            KeyCode::Backspace => {
                self.input.pop();
                self.command_deck_follow_latest = true;
            }
            KeyCode::Tab => {
                self.active_pane = match self.active_pane {
                    ActivePane::Main => ActivePane::Telemetry,
                    ActivePane::Telemetry => ActivePane::SystemErrors,
                    ActivePane::SystemErrors => ActivePane::CommandDeck,
                    ActivePane::CommandDeck => ActivePane::Main,
                };
            }
            KeyCode::Up | KeyCode::PageUp => {
                match self.active_pane {
                    ActivePane::Main => {
                        self.chat_follow_latest = false;
                        self.chat_scroll = self.chat_scroll.saturating_sub(1);
                    }
                    ActivePane::Telemetry => self.telemetry_scroll = self.telemetry_scroll.saturating_sub(1),
                    ActivePane::SystemErrors => self.system_errors_scroll = self.system_errors_scroll.saturating_sub(1),
                    ActivePane::CommandDeck => {
                        self.command_deck_follow_latest = false;
                        self.command_deck_scroll = self.command_deck_scroll.saturating_sub(1);
                    }
                }
            }
            KeyCode::Down | KeyCode::PageDown => {
                match self.active_pane {
                    ActivePane::Main => {
                        self.chat_follow_latest = false;
                        self.chat_scroll = self.chat_scroll.saturating_add(1);
                    }
                    ActivePane::Telemetry => self.telemetry_scroll = self.telemetry_scroll.saturating_add(1),
                    ActivePane::SystemErrors => self.system_errors_scroll = self.system_errors_scroll.saturating_add(1),
                    ActivePane::CommandDeck => {
                        self.command_deck_follow_latest = false;
                        self.command_deck_scroll = self.command_deck_scroll.saturating_add(1);
                    }
                }
            }
            KeyCode::End => {
                match self.active_pane {
                    ActivePane::Main => self.chat_follow_latest = true,
                    ActivePane::CommandDeck => self.command_deck_follow_latest = true,
                    _ => {}
                }
            }
            _ => {}
        }
    }
}
