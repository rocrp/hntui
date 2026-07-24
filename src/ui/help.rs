use crate::app::{App, View};
use crate::ui::theme;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < 10 || area.height < 6 {
        return;
    }

    fn section_title(name: &str, active: bool) -> Line<'static> {
        Line::from(Span::styled(
            name.to_string(),
            theme::section_heading(active),
        ))
    }

    fn kv(keys: &str, desc: &str) -> Line<'static> {
        Line::from(vec![
            Span::styled(format!("  {keys}"), theme::KEY),
            Span::styled(format!(": {desc}"), theme::LABEL),
        ])
    }

    let active = app.view;
    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled("Shortcuts", theme::HEADER)));
    lines.push(Line::from(Span::styled(
        "Press ? or Esc to close.",
        theme::HINT,
    )));
    lines.push(Line::raw(""));

    let stories_active = active == View::Stories;
    lines.push(section_title("Stories", stories_active));
    lines.push(kv("j/k, ↓/↑", "move"));
    lines.push(kv("gg, G", "top / bottom"));
    lines.push(kv("Ctrl+d / Ctrl+u", "page down / up"));
    lines.push(kv("Enter / Space / l / →", "open comments"));
    lines.push(kv("o", "open source link (browser)"));
    lines.push(kv("O", "open comments page (browser)"));
    lines.push(kv("/", "search stories"));
    lines.push(kv("f", "switch feed"));
    lines.push(kv("F", "filter by title"));
    lines.push(kv("r", "refresh"));
    lines.push(kv(",", "settings"));
    lines.push(kv("q / Esc", "quit"));
    lines.push(Line::raw(""));

    let comments_active = active == View::Comments;
    lines.push(section_title("Comments", comments_active));
    lines.push(kv("j/k, ↓/↑", "move"));
    lines.push(kv("gg, G", "top / bottom"));
    lines.push(kv("Ctrl+d / Ctrl+u", "page down / up"));
    lines.push(kv("h / ←", "collapse thread"));
    lines.push(kv("l / →", "expand thread (loads children)"));
    lines.push(kv("Enter / c", "toggle collapse/expand"));
    lines.push(kv("y", "copy selected comment to clipboard"));
    lines.push(kv("o", "open comments page (browser)"));
    lines.push(kv("O", "open source link (browser)"));
    lines.push(kv("s", "summarize (AI)"));
    lines.push(kv("r", "refresh"));
    lines.push(kv(",", "settings"));
    lines.push(kv("q / Esc", "back"));
    lines.push(Line::raw(""));

    lines.push(section_title("Summary", app.summary_overlay.is_visible()));
    lines.push(kv("gg, G", "top / bottom"));

    let desired_width = area.width.min(76);
    let desired_height = (lines.len() as u16).saturating_add(2).min(area.height);
    let popup = super::centered(area, desired_width, desired_height);

    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled("?", theme::HEADER));
    let paragraph = Paragraph::new(Text::from(lines))
        .wrap(Wrap { trim: true })
        .block(block)
        .style(theme::POPUP);
    frame.render_widget(paragraph, popup);
}
