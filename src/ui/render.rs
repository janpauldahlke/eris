use ratatui::{
    layout::{Layout, Constraint, Direction},
    widgets::{Block, Borders, Paragraph, Wrap},
    style::{Style, Color, Modifier},
    text::{Line, Span, Text},
    Frame,
};
use crate::ui::TuiApp;
use crate::orchestrator::state::AgentState;
use crate::ui::app::ActivePane;

pub fn draw(f: &mut Frame, app: &TuiApp) {
    let background = Block::default().style(Style::default().bg(Color::Rgb(8, 10, 18)));
    f.render_widget(background, f.size());

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

    let get_border_style = |pane: ActivePane| {
        if app.active_pane == pane {
            Style::default().fg(Color::Rgb(92, 229, 190))
        } else {
            Style::default().fg(Color::Rgb(63, 70, 95))
        }
    };

    let stars = ["* .  .  *", ".  *  .  ", "  .  *  .", ".  .  *  "];
    let star_idx = (app.tick_count as usize / 2) % stars.len();
    let title = format!(" ERIS Console {} ", stars[star_idx]);

    // Zone 1: Main Viewport
    let mut chat_lines: Vec<Line> = Vec::new();
    for msg in &app.chat_stack {
        if let Some(rest) = msg.strip_prefix("You: ") {
            chat_lines.push(Line::from(vec![
                Span::styled("You: ", Style::default().fg(Color::Rgb(120, 180, 255)).add_modifier(Modifier::BOLD)),
                Span::styled(rest.to_string(), Style::default().fg(Color::Rgb(214, 223, 255))),
            ]));
        } else if msg.starts_with('[') && msg.contains("]: ") {
            let split_idx = msg.find("]: ").unwrap_or(0);
            let (name_part, rest_part) = msg.split_at(split_idx + 3);
            chat_lines.push(Line::from(vec![
                Span::styled(name_part.to_string(), Style::default().fg(Color::Rgb(140, 255, 220)).add_modifier(Modifier::BOLD)),
                Span::styled(rest_part.to_string(), Style::default().fg(Color::Rgb(245, 248, 255))),
            ]));
        } else {
            chat_lines.push(Line::from(Span::styled(
                msg.clone(),
                Style::default().fg(Color::Rgb(180, 186, 212)),
            )));
        }
        chat_lines.push(Line::default());
    }

    let chat = Paragraph::new(Text::from(chat_lines))
        .style(Style::default().bg(Color::Rgb(10, 13, 24)))
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(get_border_style(ActivePane::Main))
            .title(title))
        .wrap(Wrap { trim: true })
        .scroll((app.chat_scroll, 0));
    f.render_widget(chat, top_chunks[0]);

    // Zone 2 & 3: Pulse / Telemetry
    let phase = (app.tick_count as usize / 2) % 4;
    let pulse_str = match app.state.state {
        AgentState::Idle => {
            let frames = ["[ - _ - ] (Idle)", "[ . _ . ] (Idle)", "[ - _ - ] (Idle)", "[ . _ . ] (Idle)"];
            frames[phase]
        }
        AgentState::Chat => {
            let frames = ["[ ^ _ ^ ] (Chat)", "[ ^ o ^ ] (Chat)", "[ ^ _ ^ ] (Chat)", "[ ^ o ^ ] (Chat)"];
            frames[phase]
        }
        AgentState::Reflect => {
            let frames = ["[ ~ _ ~ ] (Reflect)", "[ * _ * ] (Reflect)", "[ ~ _ ~ ] (Reflect)", "[ * _ * ] (Reflect)"];
            frames[phase]
        }
        AgentState::Recover => {
            let frames = ["[ O _ O ] (Recover)", "[ X _ X ] (Recover)", "[ O _ O ] (Recover)", "[ X _ X ] (Recover)"];
            frames[phase]
        }
    };
    
    let color = match app.state.state {
        AgentState::Idle => Color::Rgb(120, 180, 255),
        AgentState::Chat => Color::Rgb(92, 229, 190),
        AgentState::Reflect => Color::Rgb(255, 209, 102),
        AgentState::Recover => Color::Rgb(255, 107, 129),
    };

    let telemetry = format!(
        "{}\n\nTool Rounds: {}/5\nRecoveries: {}/3",
        pulse_str, app.state.tool_rounds, app.state.recovery_count
    );
    let sidebar = Paragraph::new(telemetry)
        .style(Style::default().fg(color).bg(Color::Rgb(14, 16, 28)).add_modifier(Modifier::BOLD))
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(get_border_style(ActivePane::Telemetry))
            .title(" Pulse & Telemetry "))
        .scroll((app.telemetry_scroll, 0));
    f.render_widget(sidebar, top_chunks[1]);

    // Zone 3.5: System Errors
    let sys_errors_text = app.system_messages.join("\n");
    let sys_errors = Paragraph::new(sys_errors_text)
        .style(Style::default().fg(Color::Rgb(255, 132, 132)).bg(Color::Rgb(14, 16, 28)))
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(get_border_style(ActivePane::SystemErrors))
            .title(" System Errors / Telemetry "))
        .wrap(Wrap { trim: true })
        .scroll((app.system_errors_scroll, 0));
    f.render_widget(sys_errors, chunks[1]);

    // Zone 4: Input / Command Deck
    let input = Paragraph::new(app.input.as_str())
        .style(Style::default().fg(Color::Rgb(214, 223, 255)).bg(Color::Rgb(8, 10, 18)))
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(120, 180, 255)))
            .title(" Command Deck "));
    f.render_widget(input, chunks[2]);

    let input_chunk = chunks[2];
    f.set_cursor(input_chunk.x + app.input.len() as u16 + 1, input_chunk.y + 1);
}
