use crate::app::{App, ConnectionTestState, SettingsPopup, SettingsRow};
use crate::ui::theme;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;
use std::time::Duration;

const MAX_POPUP_WIDTH: u16 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SettingsAreas {
    popup: Rect,
    content: Rect,
}

pub fn render(frame: &mut Frame, app: &App) {
    let Some(popup) = &app.settings_popup else {
        return;
    };
    render_popup(frame, popup);
}

fn render_popup(frame: &mut Frame, popup: &SettingsPopup) {
    let Some(areas) = settings_areas(frame.area(), popup) else {
        return;
    };

    frame.render_widget(Clear, areas.popup);
    frame.render_widget(popup_block().style(theme::POPUP), areas.popup);
    frame.render_widget(settings_paragraph(popup).style(theme::POPUP), areas.content);
}

fn settings_paragraph(popup: &SettingsPopup) -> Paragraph<'static> {
    let fields = SettingsPopup::fields();
    let max_label_len = fields
        .iter()
        .map(|field| field.label().len())
        .max()
        .unwrap_or(0);

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled("Settings", theme::HEADER)));
    lines.push(Line::raw(""));

    for (i, row) in SettingsPopup::rows().iter().copied().enumerate() {
        let is_cursor = i == popup.cursor;
        let marker = if is_cursor { "> " } else { "  " };
        match row {
            SettingsRow::Field(field) => {
                let is_editing = is_cursor && popup.editing;
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
                    let (cursor_char, after) = if pos < chars.len() {
                        (
                            chars[pos].to_string(),
                            chars[pos + 1..].iter().collect::<String>(),
                        )
                    } else {
                        (" ".to_string(), String::new())
                    };

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
            SettingsRow::TestConnection => {
                let style = if is_cursor {
                    theme::ACCENT
                } else {
                    theme::LABEL
                };
                lines.push(Line::from(Span::styled(
                    format!("{marker}[ Test connection ]"),
                    style,
                )));
            }
        }
    }

    if let Some(status) = &popup.api_key_status {
        lines.push(Line::from(Span::styled(format!("  {status}"), theme::HINT)));
    }

    let endpoint = popup.resolved_endpoint_preview();
    let endpoint_style = if endpoint.is_error() {
        theme::WARN
    } else {
        theme::HINT
    };
    lines.push(Line::from(Span::styled(
        format!("  {}", endpoint.text()),
        endpoint_style,
    )));

    match &popup.connection_test {
        ConnectionTestState::Idle => {}
        ConnectionTestState::Testing => {
            lines.push(Line::from(Span::styled("  ⏳ testing…", theme::HINT)))
        }
        ConnectionTestState::Success { model, ttft } => {
            lines.push(Line::from(Span::styled(
                format!("  ✓ ok · {model} · {}", format_ttft(*ttft)),
                theme::SUCCESS,
            )));
        }
        ConnectionTestState::Error(message) => lines.push(Line::from(Span::styled(
            format!("  ✗ {message}"),
            theme::ERROR,
        ))),
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
            Span::styled(":activate  ", theme::HINT),
            Span::styled("Esc/q", theme::KEY),
            Span::styled(":close", theme::HINT),
        ]));
    }

    Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false })
}

fn format_ttft(ttft: Duration) -> String {
    if ttft < Duration::from_secs(1) {
        format!("{}ms", ttft.as_millis())
    } else {
        format!("{:.1}s", ttft.as_secs_f64())
    }
}

fn popup_block() -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(",", theme::HEADER))
}

fn settings_areas(area: Rect, popup: &SettingsPopup) -> Option<SettingsAreas> {
    if area.width < 20 || area.height < 10 {
        return None;
    }
    let desired_width = area.width.min(MAX_POPUP_WIDTH);
    let sizing_popup = Rect::new(0, 0, desired_width, area.height);
    let content_width = popup_block().inner(sizing_popup).width;
    let content_height =
        u16::try_from(settings_paragraph(popup).line_count(content_width)).unwrap_or(u16::MAX);
    let desired_height = content_height.saturating_add(2).min(area.height);
    let popup = super::centered(area, desired_width, desired_height);
    let content = popup_block().inner(popup);
    Some(SettingsAreas { popup, content })
}

pub(crate) fn popup_rect(area: Rect, popup: &SettingsPopup) -> Option<Rect> {
    settings_areas(area, popup).map(|areas| areas.popup)
}

#[cfg(test)]
mod render_tests;
