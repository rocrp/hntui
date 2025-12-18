use crate::app::{App, View};
use crate::ui::theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < 10 || area.height < 6 {
        return;
    }

    let header_style = Style::default()
        .fg(theme::TEXT)
        .add_modifier(Modifier::BOLD);
    let hint_style = Style::default().fg(theme::SUBTEXT1);
    let key_style = Style::default()
        .fg(theme::TEXT)
        .add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(theme::SUBTEXT1);

    fn section_title(name: &str, active: bool) -> Line<'static> {
        let style = if active {
            Style::default()
                .fg(theme::MAUVE)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(theme::SUBTEXT0)
                .add_modifier(Modifier::BOLD)
        };
        Line::from(Span::styled(name.to_string(), style))
    }

    fn kv(keys: &str, desc: &str, key_style: Style, desc_style: Style) -> Line<'static> {
        Line::from(vec![
            Span::styled(format!("  {keys}"), key_style),
            Span::styled(format!(": {desc}"), desc_style),
        ])
    }

    let active = app.view;
    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled("Shortcuts", header_style)));
    lines.push(Line::from(Span::styled(
        "Press ? or Esc to close.",
        hint_style,
    )));
    lines.push(Line::raw(""));

    let stories_active = active == View::Stories;
    lines.push(section_title("Stories", stories_active));
    lines.push(kv("j/k, ↓/↑", "move", key_style, desc_style));
    lines.push(kv("gg, G", "top / bottom", key_style, desc_style));
    lines.push(kv(
        "Ctrl+d / Ctrl+u",
        "page down / up",
        key_style,
        desc_style,
    ));
    lines.push(kv(
        "Enter / Space / l / →",
        "open comments",
        key_style,
        desc_style,
    ));
    lines.push(kv("o", "open source link (browser)", key_style, desc_style));
    lines.push(kv("r", "refresh", key_style, desc_style));
    lines.push(kv("q / Esc", "quit", key_style, desc_style));
    lines.push(Line::raw(""));

    let comments_active = active == View::Comments;
    lines.push(section_title("Comments", comments_active));
    lines.push(kv("j/k, ↓/↑", "move", key_style, desc_style));
    lines.push(kv("gg, G", "top / bottom", key_style, desc_style));
    lines.push(kv(
        "Ctrl+d / Ctrl+u",
        "page down / up",
        key_style,
        desc_style,
    ));
    lines.push(kv("h / ←", "collapse thread", key_style, desc_style));
    lines.push(kv(
        "l / →",
        "expand thread (loads children)",
        key_style,
        desc_style,
    ));
    lines.push(kv("c", "toggle collapse/expand", key_style, desc_style));
    lines.push(kv("o", "open source link (browser)", key_style, desc_style));
    lines.push(kv("r", "refresh", key_style, desc_style));
    lines.push(kv("q / Esc", "back", key_style, desc_style));

    let desired_width = area.width.min(76);
    let desired_height = (lines.len() as u16).saturating_add(2).min(area.height);
    let popup = centered(area, desired_width, desired_height);

    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled("?", header_style));
    let paragraph = Paragraph::new(Text::from(lines))
        .wrap(Wrap { trim: true })
        .block(block)
        .style(Style::default().bg(theme::SURFACE2));
    frame.render_widget(paragraph, popup);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x.saturating_add(area.width.saturating_sub(width) / 2);
    let y = area
        .y
        .saturating_add(area.height.saturating_sub(height) / 2);
    Rect {
        x,
        y,
        width,
        height,
    }
}
