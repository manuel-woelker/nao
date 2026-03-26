use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders};

pub(crate) fn top_level_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(area)
        .to_vec()
}

pub(crate) fn focused_block<'a>(title: &'a str, focused: bool) -> Block<'a> {
    let block = Block::default().borders(Borders::ALL).title(title);
    if focused {
        block.border_style(Style::default().add_modifier(Modifier::BOLD))
    } else {
        block
    }
}

pub(crate) fn centered_rect(area: Rect, width_percent: u16, height_percent: u16) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height_percent) / 2),
            Constraint::Percentage(height_percent),
            Constraint::Percentage((100 - height_percent) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100 - width_percent) / 2),
        ])
        .split(vertical[1])[1]
}

pub(crate) fn adjust_scroll(current: u16, delta: i32, max: u16) -> u16 {
    let next = current as i32 + delta;
    next.clamp(0, max as i32) as u16
}

pub(crate) fn spinner_frames() -> &'static [&'static str] {
    &["⠋ ", "⠙ ", "⠹ ", "⠸ ", "⠼ ", "⠴ ", "⠦ ", "⠧ ", "⠇ ", "⠏ "]
}

pub(crate) fn render_task_state_emoji(status: &str, spinner_frame: usize) -> &'static str {
    match status {
        "pending" => "⚪",
        "running" => spinner_frames()[spinner_frame % spinner_frames().len()],
        "completed" => "✅",
        "failed" => "❌",
        "skipped" => "⏭ ",
        _ => "⚪",
    }
}

pub(crate) fn pretty_duration(duration_nanos: u128) -> String {
    if duration_nanos < 1_000 {
        return format!("{duration_nanos}ns");
    }
    if duration_nanos < 1_000_000 {
        return format!("{:.1}us", duration_nanos as f64 / 1_000.0);
    }
    if duration_nanos < 1_000_000_000 {
        return format!("{:.1}ms", duration_nanos as f64 / 1_000_000.0);
    }
    format!("{:.1}s", duration_nanos as f64 / 1_000_000_000.0)
}
