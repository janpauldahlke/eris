use ratatui::{
    layout::{Layout, Constraint, Direction},
    widgets::{Block, Borders, Paragraph, List, ListItem},
    style::{Style, Color, Modifier},
    Frame,
};
use crate::ui::TuiApp;
use crate::orchestrator::state::AgentState;

pub fn draw(f: &mut Frame, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(f.size());

    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(chunks[0]);

    // Zone 1: Main Viewport
    let messages: Vec<ListItem> = app.chat_stack.iter()
        .map(|m| ListItem::new(m.as_str()))
        .collect();
    let chat = List::new(messages).block(Block::default().borders(Borders::ALL).title(" Primary Viewport "));
    f.render_widget(chat, top_chunks[0]);

    // Zone 2 & 3: Pulse / Telemetry
    let pulse_str = match app.state.state {
        AgentState::Idle => "[ - _ - ] (Idle)",
        AgentState::Chat => "[ ^ _ ^ ] (Chat)",
        AgentState::Reflect => "[ ~ _ ~ ] (Reflect)",
        AgentState::Recover => "[ O _ O ] (Recover)",
    };
    
    let color = match app.state.state {
        AgentState::Idle => Color::Blue,
        AgentState::Chat => Color::Green,
        AgentState::Reflect => Color::Yellow,
        AgentState::Recover => Color::Red,
    };

    let telemetry = format!(
        "{}\n\nTool Rounds: {}/5\nRecoveries: {}/3",
        pulse_str, app.state.tool_rounds, app.state.recovery_count
    );
    let sidebar = Paragraph::new(telemetry)
        .style(Style::default().fg(color).add_modifier(Modifier::BOLD))
        .block(Block::default().borders(Borders::ALL).title(" Pulse & Telemetry "));
    f.render_widget(sidebar, top_chunks[1]);

    // Zone 4: Input / Command Deck
    let input = Paragraph::new(app.input.as_str())
        .block(Block::default().borders(Borders::ALL).title(" Command Deck "));
    f.render_widget(input, chunks[1]);
}
