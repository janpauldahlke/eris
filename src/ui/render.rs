use ratatui::{
    layout::{Layout, Constraint, Direction},
    widgets::{Block, Borders, Paragraph, Wrap},
    style::{Style, Color, Modifier},
    text::{Line, Span, Text},
    Frame,
};
use crate::ui::TuiApp;
use crate::engine::token_metrics::LlmTokenSnapshot;
use crate::orchestrator::state::AgentState;
use crate::ui::app::ActivePane;

const STATUS_ACTIVITY_MAX_CHARS: usize = 100;

fn truncate_status_line(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    let mut out = s.chars().take(max_chars).collect::<String>();
    out.push('…');
    out
}

fn push_multiline(
    chat_lines: &mut Vec<Line>,
    prefix: Option<(String, Style)>,
    content: &str,
    style: Style,
) {
    let mut lines = content.lines();
    if let Some(first) = lines.next() {
        if let Some((label, label_style)) = prefix.clone() {
            chat_lines.push(Line::from(vec![
                Span::styled(label, label_style),
                Span::styled(first.to_string(), style),
            ]));
        } else {
            chat_lines.push(Line::from(Span::styled(first.to_string(), style)));
        }
        for line in lines {
            chat_lines.push(Line::from(Span::styled(line.to_string(), style)));
        }
    } else if let Some((label, label_style)) = prefix {
        chat_lines.push(Line::from(vec![Span::styled(label, label_style)]));
    }
}

fn wrap_input_lines(input: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }

    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;

    for word in input.split_whitespace() {
        let word_len = word.chars().count();
        let separator_len = if current_len > 0 { 1 } else { 0 };

        if current_len + separator_len + word_len <= width {
            if separator_len == 1 {
                current.push(' ');
                current_len += 1;
            }
            current.push_str(word);
            current_len += word_len;
            continue;
        }

        if !current.is_empty() {
            lines.push(current);
            current = String::new();
        }

        if word_len <= width {
            current.push_str(word);
            current_len = word_len;
        } else {
            let mut chunk = String::new();
            let mut chunk_len = 0usize;
            for ch in word.chars() {
                chunk.push(ch);
                chunk_len += 1;
                if chunk_len == width {
                    lines.push(chunk);
                    chunk = String::new();
                    chunk_len = 0;
                }
            }
            current = chunk;
            current_len = chunk_len;
        }
    }

    if !current.is_empty() {
        lines.push(current);
    }

    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}

pub fn draw(f: &mut Frame, app: &TuiApp, llm_tokens: &LlmTokenSnapshot) {
    let background = Block::default().style(Style::default().bg(Color::Rgb(8, 10, 18)));
    f.render_widget(background, f.size());

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),        // ERIS Console (grows with free space)
            Constraint::Length(8),     // Telemetry + Status bar
            Constraint::Length(8),     // Command Deck (~6 inner lines)
        ])
        .split(f.size());

    let bottom_bar = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(75), Constraint::Percentage(25)])
        .split(chunks[1]);

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

    // ── Chat viewport (full width) ───────────────────────────────
    let mut chat_lines: Vec<Line> = Vec::new();
    for msg in &app.chat_stack {
        if let Some(rest) = msg.strip_prefix("You: ") {
            push_multiline(
                &mut chat_lines,
                Some((
                    "You: ".to_string(),
                    Style::default().fg(Color::Rgb(120, 180, 255)).add_modifier(Modifier::BOLD),
                )),
                rest,
                Style::default().fg(Color::Rgb(214, 223, 255)),
            );
        } else if msg.starts_with('[') && msg.contains("]: ") {
            let split_idx = msg.find("]: ").unwrap_or(0);
            let (name_part, rest_part) = msg.split_at(split_idx + 3);
            push_multiline(
                &mut chat_lines,
                Some((
                    name_part.to_string(),
                    Style::default().fg(Color::Rgb(140, 255, 220)).add_modifier(Modifier::BOLD),
                )),
                rest_part.trim_start(),
                Style::default().fg(Color::Rgb(245, 248, 255)),
            );
        } else {
            push_multiline(
                &mut chat_lines,
                None,
                msg,
                Style::default().fg(Color::Rgb(180, 186, 212)),
            );
        }
        chat_lines.push(Line::default());
    }

    let chat_inner_height = chunks[0].height.saturating_sub(2) as usize;
    if chat_lines.len() < chat_inner_height {
        let pad_count = chat_inner_height - chat_lines.len();
        let mut padded_lines = Vec::with_capacity(chat_inner_height);
        for _ in 0..pad_count {
            padded_lines.push(Line::default());
        }
        padded_lines.extend(chat_lines);
        chat_lines = padded_lines;
    }
    let max_chat_scroll = chat_lines.len().saturating_sub(chat_inner_height) as u16;
    let chat_scroll = if app.chat_follow_latest {
        max_chat_scroll
    } else {
        app.chat_scroll.min(max_chat_scroll)
    };

    let chat = Paragraph::new(Text::from(chat_lines))
        .style(Style::default().bg(Color::Rgb(10, 13, 24)))
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(get_border_style(ActivePane::Main))
            .title(title))
        .wrap(Wrap { trim: true })
        .scroll((chat_scroll, 0));
    f.render_widget(chat, chunks[0]);

    // ── Telemetry / System log (75%) ─────────────────────────────
    let sys_text = if app.system_messages.is_empty() {
        String::from("No system events.")
    } else {
        app.system_messages.join("\n")
    };
    let telemetry = Paragraph::new(sys_text)
        .style(Style::default().fg(Color::Rgb(180, 186, 212)).bg(Color::Rgb(14, 16, 28)))
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(get_border_style(ActivePane::Telemetry))
            .title(" Telemetry "))
        .wrap(Wrap { trim: true })
        .scroll((app.telemetry_scroll, 0));
    f.render_widget(telemetry, bottom_bar[0]);

    // ── Status / Pulse (25%) ─────────────────────────────────────
    let phase = (app.tick_count as usize / 2) % 4;
    let queued = app.state.queued_inputs.max(app.pending_inputs);
    let is_queued_idle = queued > 0 && app.state.state == AgentState::Idle;

    let (pulse_str, state_label, state_color): (&str, &str, Color) = if is_queued_idle {
        let frames = ["[ … ]", "[ · ]", "[ … ]", "[ · ]"];
        (frames[phase], "Queued", Color::Rgb(200, 190, 120))
    } else {
        let pulse_str = match app.state.state {
            AgentState::Idle => {
                let frames = ["[ - _ - ]", "[ . _ . ]", "[ - _ - ]", "[ . _ . ]"];
                frames[phase]
            }
            AgentState::Chat => {
                let frames = ["[ ^ _ ^ ]", "[ ^ o ^ ]", "[ ^ _ ^ ]", "[ ^ o ^ ]"];
                frames[phase]
            }
            AgentState::Reflect => {
                let frames = ["[ ~ _ ~ ]", "[ * _ * ]", "[ ~ _ ~ ]", "[ * _ * ]"];
                frames[phase]
            }
            AgentState::Recover => {
                let frames = ["[ O _ O ]", "[ X _ X ]", "[ O _ O ]", "[ X _ X ]"];
                frames[phase]
            }
        };

        let state_color = match app.state.state {
            AgentState::Idle => Color::Rgb(120, 180, 255),
            AgentState::Chat => Color::Rgb(92, 229, 190),
            AgentState::Reflect => Color::Rgb(255, 209, 102),
            AgentState::Recover => Color::Rgb(255, 107, 129),
        };

        let state_label = match app.state.state {
            AgentState::Idle => "Idle",
            AgentState::Chat => "Chat",
            AgentState::Reflect => "Reflect",
            AgentState::Recover => "Recover",
        };

        (pulse_str, state_label, state_color)
    };

    let activity_line = app
        .state
        .activity_line
        .as_deref()
        .map(|s| truncate_status_line(s, STATUS_ACTIVITY_MAX_CHARS));

    let status_text = if let Some(ref act) = activity_line {
        format!(
            "{}\n{}\n{}\nT:{}/5 R:{}/3\nQ:{}\nrt:{}ms llm:{}ms\ntool:{}ms total:{}ms\nmatch:{}\nollama tok: p{} g{} sum{}",
            pulse_str,
            state_label,
            act,
            app.state.tool_rounds,
            app.state.recovery_count,
            queued,
            app.state.router_ms,
            app.state.llm_ms,
            app.state.tool_ms,
            app.state.total_ms,
            app.state.top_tool_match.as_deref().unwrap_or("-"),
            llm_tokens.prompt_tokens,
            llm_tokens.generated_tokens,
            llm_tokens.total(),
        )
    } else {
        format!(
            "{}\n{}\nT:{}/5 R:{}/3\nQ:{}\nrt:{}ms llm:{}ms\ntool:{}ms total:{}ms\nmatch:{}\nollama tok: p{} g{} sum{}",
            pulse_str,
            state_label,
            app.state.tool_rounds,
            app.state.recovery_count,
            queued,
            app.state.router_ms,
            app.state.llm_ms,
            app.state.tool_ms,
            app.state.total_ms,
            app.state.top_tool_match.as_deref().unwrap_or("-"),
            llm_tokens.prompt_tokens,
            llm_tokens.generated_tokens,
            llm_tokens.total(),
        )
    };
    let status = Paragraph::new(status_text)
        .style(Style::default().fg(state_color).bg(Color::Rgb(14, 16, 28)).add_modifier(Modifier::BOLD))
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(get_border_style(ActivePane::SystemErrors))
            .title(" Status "));
    f.render_widget(status, bottom_bar[1]);

    // ── Input / Command Deck (minimal chrome; lifecycle lives in Status) ─
    let deck_frames = ["-", "\\", "|", "/"];
    let deck_mini = deck_frames[phase % 4];

    let input_chunk = chunks[2];
    let inner_width = input_chunk.width.saturating_sub(2) as usize;
    let inner_height = input_chunk.height.saturating_sub(2) as usize;
    let wrapped_lines = wrap_input_lines(app.input.as_str(), inner_width);
    let max_input_scroll = wrapped_lines.len().saturating_sub(inner_height) as u16;
    let input_scroll = if app.command_deck_follow_latest {
        max_input_scroll
    } else {
        app.command_deck_scroll.min(max_input_scroll)
    };
    let cursor_line_idx = wrapped_lines.len().saturating_sub(1);
    let cursor_col = wrapped_lines
        .last()
        .map(|line| line.chars().count() as u16)
        .unwrap_or(0);
    let visible_cursor_row = cursor_line_idx.saturating_sub(input_scroll as usize) as u16;

    let input = Paragraph::new(Text::from(
        wrapped_lines
            .iter()
            .map(|line| Line::from(line.as_str()))
            .collect::<Vec<Line>>(),
    ))
        .style(Style::default().fg(Color::Rgb(214, 223, 255)).bg(Color::Rgb(8, 10, 18)))
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(get_border_style(ActivePane::CommandDeck))
            .title(Line::from(vec![
                Span::styled(" Cmd ", Style::default().fg(Color::Rgb(120, 180, 255)).add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!(" {} ", deck_mini),
                    Style::default().fg(Color::Rgb(100, 110, 140)).add_modifier(Modifier::BOLD),
                ),
            ])))
        .scroll((input_scroll, 0))
        .wrap(Wrap { trim: false });
    f.render_widget(input, chunks[2]);

    f.set_cursor(input_chunk.x + cursor_col + 1, input_chunk.y + visible_cursor_row + 1);
}
