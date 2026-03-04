use crate::app::{App, SettingsPopup};
use crate::ui::theme;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;
use std::time::Duration;

pub fn render(frame: &mut Frame, app: &App) {
    let Some(popup) = &app.settings_popup else {
        return;
    };
    let area = frame.area();
    if area.width < 20 || area.height < 10 {
        return;
    }

    let header_style = Style::default()
        .fg(theme::palette().text)
        .add_modifier(Modifier::BOLD);
    let hint_style = Style::default().fg(theme::palette().subtext0);
    let key_style = Style::default()
        .fg(theme::palette().text)
        .add_modifier(Modifier::BOLD);
    let label_style = Style::default().fg(theme::palette().subtext1);
    let value_style = Style::default().fg(theme::palette().text);
    let cursor_style = Style::default()
        .fg(theme::palette().mauve)
        .add_modifier(Modifier::BOLD);
    let editing_style = Style::default()
        .fg(theme::palette().green)
        .add_modifier(Modifier::BOLD);

    let labels = popup.field_labels();
    let values = popup.field_values();
    let max_label_len = labels.iter().map(|l| l.len()).max().unwrap_or(0);

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled("Settings", header_style)));
    lines.push(Line::raw(""));

    for i in 0..SettingsPopup::FIELD_COUNT {
        let is_cursor = i == popup.cursor;
        let is_editing = is_cursor && popup.editing;
        let marker = if is_cursor { "> " } else { "  " };
        let padded_label = format!("{:width$}", labels[i], width = max_label_len);

        let display_value = if is_editing {
            // Handled below with cursor spans
            String::new()
        } else if i == 2 && !values[i].is_empty() {
            // Mask API key
            let v = values[i];
            if v.len() > 4 {
                format!("{}...{}", &v[..2], &v[v.len() - 2..])
            } else {
                "*".repeat(v.len())
            }
        } else {
            values[i].to_string()
        };

        let style = if is_editing {
            editing_style
        } else if is_cursor {
            cursor_style
        } else {
            label_style
        };

        if is_editing {
            let buf = &popup.edit_buffer;
            let pos = popup.edit_cursor;
            let chars: Vec<char> = buf.chars().collect();
            let before: String = chars[..pos].iter().collect();
            let cursor_char: String;
            let after: String;

            let block_cursor_style = Style::default()
                .fg(theme::palette().surface2)
                .bg(theme::palette().green);

            if pos < chars.len() {
                cursor_char = chars[pos].to_string();
                after = chars[pos + 1..].iter().collect();
            } else {
                cursor_char = " ".to_string();
                after = String::new();
            }

            lines.push(Line::from(vec![
                Span::styled(format!("{marker}{padded_label}: "), style),
                Span::styled(before, editing_style),
                Span::styled(cursor_char, block_cursor_style),
                Span::styled(after, editing_style),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(format!("{marker}{padded_label}: "), style),
                Span::styled(display_value, value_style),
            ]));
        }
    }

    lines.push(Line::raw(""));

    // "Saved!" flash
    let show_saved = popup
        .saved_at
        .is_some_and(|t| t.elapsed() < Duration::from_secs(2));

    if show_saved {
        lines.push(Line::from(vec![
            Span::styled(
                "Saved! ",
                Style::default()
                    .fg(theme::palette().green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Esc/q", key_style),
            Span::styled(":close", hint_style),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("j/k", key_style),
            Span::styled(":nav  ", hint_style),
            Span::styled("Enter", key_style),
            Span::styled(":edit  ", hint_style),
            Span::styled("Esc/q", key_style),
            Span::styled(":close", hint_style),
        ]));
    }

    let desired_width = area.width.min(60);
    let desired_height = (lines.len() as u16).saturating_add(2).min(area.height);
    let popup_rect = super::centered(area, desired_width, desired_height);

    frame.render_widget(Clear, popup_rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(",", header_style));
    let paragraph = Paragraph::new(Text::from(lines))
        .block(block)
        .style(Style::default().bg(theme::palette().surface2));
    frame.render_widget(paragraph, popup_rect);
}
