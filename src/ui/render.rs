use ratatui::{
    layout::{Layout, Constraint, Direction},
    widgets::{Block, Borders, Paragraph, Wrap},
    style::{Style, Color, Modifier},
    Frame,
};
use crate::ui::TuiApp;
use crate::orchestrator::state::AgentState;

pub fn draw(f: &mut Frame, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0), // Main
            Constraint::Percentage(20), // System Errors
            Constraint::Min(3), // Input
        ])
        .split(f.size());

    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(chunks[0]);

    // Zone 1: Main Viewport
    let chat_text = app.chat_stack.join("\n\n");
    let chat = Paragraph::new(chat_text)
        .block(Block::default().borders(Borders::ALL).title(" Primary Viewport "))
        .wrap(Wrap { trim: true })
        .scroll((app.viewport_scroll, 0));
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

    // Zone 3.5: System Errors
    let sys_errors_text = app.system_messages.join("\n");
    let sys_errors = Paragraph::new(sys_errors_text)
        .style(Style::default().fg(Color::Red))
        .block(Block::default().borders(Borders::ALL).title(" System Errors / Telemetry "))
        .wrap(Wrap { trim: true });
    f.render_widget(sys_errors, chunks[1]);

    // Zone 4: Input / Command Deck
    let input = Paragraph::new(app.input.as_str())
        .block(Block::default().borders(Borders::ALL).title(" Command Deck "));
    f.render_widget(input, chunks[2]);

    let input_chunk = chunks[2];
    f.set_cursor(input_chunk.x + app.input.len() as u16 + 1, input_chunk.y + 1);
}
