use crate::api::FeedKind;
use crate::app::{App, PopupFocus};
use crate::ui::theme;
use ratatui::layout::Rect;
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

    let feed_focus = popup.focus == PopupFocus::FeedList;
    let filter_focus = popup.focus == PopupFocus::FilterInput;

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled("Feed & Filter", header_style)));
    lines.push(Line::raw(""));

    // Section label
    let feed_label_style = if feed_focus { active_style } else { normal_style };
    lines.push(Line::from(Span::styled("Feed", feed_label_style)));

    // Feed list
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

    // Filter section
    let filter_label_style = if filter_focus { active_style } else { normal_style };
    lines.push(Line::from(Span::styled("Filter", filter_label_style)));

    let cursor_char = if filter_focus { "\u{2502}" } else { "" };
    let filter_line = format!("  [{}{}]", popup.filter_input, cursor_char);
    let filter_style = if filter_focus { key_style } else { normal_style };
    lines.push(Line::from(Span::styled(filter_line, filter_style)));

    if !app.keyword_filter.is_empty() {
        let visible = app.visible_story_count();
        let total = app.stories.len();
        lines.push(Line::from(Span::styled(
            format!("  {visible}/{total} visible"),
            hint_style,
        )));
    }

    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("j/k", key_style),
        Span::styled(":nav  ", hint_style),
        Span::styled("Tab", key_style),
        Span::styled(":switch  ", hint_style),
        Span::styled("Enter", key_style),
        Span::styled(":apply  ", hint_style),
        Span::styled("Esc", key_style),
        Span::styled(":close", hint_style),
    ]));

    let desired_width = area.width.min(50);
    let desired_height = (lines.len() as u16).saturating_add(2).min(area.height);
    let popup_rect = centered(area, desired_width, desired_height);

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
