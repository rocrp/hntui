use crate::app::{App, SettingsPopup};
use crate::ui::theme;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;
use std::time::Duration;

pub fn render(frame: &mut Frame, app: &App) {
    let Some(popup) = &app.settings_popup else {
        return;
    };
    let area = frame.area();
    let Some(popup_rect) = popup_rect(area) else {
        return;
    };

    let fields = SettingsPopup::fields();
    let max_label_len = fields
        .iter()
        .map(|field| field.label().len())
        .max()
        .unwrap_or(0);

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled("Settings", theme::HEADER)));
    lines.push(Line::raw(""));

    for (i, field) in fields.iter().copied().enumerate() {
        let is_cursor = i == popup.cursor;
        let is_editing = is_cursor && popup.editing;
        let marker = if is_cursor { "> " } else { "  " };
        let padded_label = format!("{:width$}", field.label(), width = max_label_len);
        let value = popup.field_value(field);

        let display_value = if is_editing {
            String::new()
        } else if field.is_secret() && !value.is_empty() {
            if value.len() > 4 {
                format!("{}...{}", &value[..2], &value[value.len() - 2..])
            } else {
                "*".repeat(value.len())
            }
        } else {
            value.to_string()
        };

        let style = if is_editing {
            theme::SUCCESS
        } else if is_cursor {
            theme::ACCENT
        } else {
            theme::LABEL
        };

        if is_editing {
            let buf = &popup.edit_buffer;
            let pos = popup.edit_cursor;
            let chars: Vec<char> = buf.chars().collect();
            let before: String = chars[..pos].iter().collect();
            let cursor_char: String;
            let after: String;

            if pos < chars.len() {
                cursor_char = chars[pos].to_string();
                after = chars[pos + 1..].iter().collect();
            } else {
                cursor_char = " ".to_string();
                after = String::new();
            }

            lines.push(Line::from(vec![
                Span::styled(format!("{marker}{padded_label}: "), style),
                Span::styled(before, theme::SUCCESS),
                Span::styled(cursor_char, theme::BLOCK_CURSOR),
                Span::styled(after, theme::SUCCESS),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(format!("{marker}{padded_label}: "), style),
                Span::styled(display_value, theme::VALUE),
            ]));
        }
    }

    lines.push(Line::raw(""));

    let show_saved = popup
        .saved_at
        .is_some_and(|t| t.elapsed() < Duration::from_secs(2));

    if show_saved {
        lines.push(Line::from(vec![
            Span::styled("Saved! ", theme::SUCCESS),
            Span::styled("Esc/q", theme::KEY),
            Span::styled(":close", theme::HINT),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("j/k", theme::KEY),
            Span::styled(":nav  ", theme::HINT),
            Span::styled("Enter", theme::KEY),
            Span::styled(":edit  ", theme::HINT),
            Span::styled("Esc/q", theme::KEY),
            Span::styled(":close", theme::HINT),
        ]));
    }

    frame.render_widget(Clear, popup_rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(",", theme::HEADER));
    let paragraph = Paragraph::new(Text::from(lines))
        .block(block)
        .style(theme::POPUP);
    frame.render_widget(paragraph, popup_rect);
}

pub(crate) fn popup_rect(area: Rect) -> Option<Rect> {
    if area.width < 20 || area.height < 10 {
        return None;
    }
    let line_count = SettingsPopup::FIELD_COUNT + 5;
    let desired_width = area.width.min(60);
    let desired_height = (line_count as u16).saturating_add(2).min(area.height);
    Some(super::centered(area, desired_width, desired_height))
}
