use crate::api::FeedKind;
use crate::app::App;
use crate::ui::theme;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

pub fn render(frame: &mut Frame, app: &App) {
    let Some(popup) = &app.feed_filter_popup else {
        return;
    };
    let area = frame.area();
    if area.width < 10 || area.height < 6 {
        return;
    }

    let header_style = Style::default()
        .fg(theme::palette().text)
        .add_modifier(Modifier::BOLD);
    let hint_style = Style::default().fg(theme::palette().subtext0);
    let key_style = Style::default()
        .fg(theme::palette().text)
        .add_modifier(Modifier::BOLD);
    let active_style = Style::default()
        .fg(theme::palette().mauve)
        .add_modifier(Modifier::BOLD);
    let normal_style = Style::default().fg(theme::palette().subtext1);

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled("Feed", header_style)));
    lines.push(Line::raw(""));

    for (i, &feed) in FeedKind::ALL.iter().enumerate() {
        let is_cursor = i == popup.feed_cursor;
        let is_current = feed == app.current_feed;
        let marker = if is_cursor { "> " } else { "  " };
        let suffix = if is_current { " *" } else { "" };
        let style = if is_cursor {
            key_style
        } else if is_current {
            active_style
        } else {
            normal_style
        };
        lines.push(Line::from(Span::styled(
            format!("{marker}{}{suffix}", feed.label()),
            style,
        )));
    }

    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("j/k", key_style),
        Span::styled(":nav  ", hint_style),
        Span::styled("Enter", key_style),
        Span::styled(":select  ", hint_style),
        Span::styled("Esc", key_style),
        Span::styled(":close", hint_style),
    ]));

    let desired_width = area.width.min(40);
    let desired_height = (lines.len() as u16).saturating_add(2).min(area.height);
    let popup_rect = super::centered(area, desired_width, desired_height);

    frame.render_widget(Clear, popup_rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled("f", header_style));
    let paragraph = Paragraph::new(Text::from(lines))
        .wrap(Wrap { trim: true })
        .block(block)
        .style(Style::default().bg(theme::palette().surface2));
    frame.render_widget(paragraph, popup_rect);
}
