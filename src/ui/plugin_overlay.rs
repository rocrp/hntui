use crate::plugin::summarize::{SummarizePlugin, SummarizeState};
use crate::ui::theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

pub fn render(frame: &mut Frame, plugin: &SummarizePlugin, spinner: char) {
    if !plugin.is_overlay_visible() {
        return;
    }

    let area = frame.area();
    if area.width < 12 || area.height < 8 {
        return;
    }

    let popup_w = (area.width * 4 / 5).max(30);
    let popup_h = (area.height * 4 / 5).max(10);
    let popup = centered(area, popup_w, popup_h);

    let header_style = Style::default()
        .fg(theme::palette().mauve)
        .add_modifier(Modifier::BOLD);
    let text_style = Style::default().fg(theme::palette().text);
    let hint_style = Style::default().fg(theme::palette().subtext0);
    let error_style = Style::default().fg(theme::palette().red);

    let state = plugin.state();

    let title = match state {
        SummarizeState::Loading => format!(" Summarizing {spinner} ({} comments) ", plugin.comment_count),
        SummarizeState::Streaming => format!(" Summarizing {spinner} "),
        SummarizeState::Done => " Summary ".to_string(),
        SummarizeState::Error => " Summary Error ".to_string(),
        SummarizeState::Idle => return,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, header_style));
    let inner = block.inner(popup);

    // Reserve 1 line for hint at bottom
    let content_height = inner.height.saturating_sub(1) as usize;

    let mut lines: Vec<Line> = Vec::new();
    match state {
        SummarizeState::Loading => {
            lines.push(Line::from(Span::styled(
                format!("Waiting for LLM response {spinner}"),
                hint_style,
            )));
        }
        SummarizeState::Streaming | SummarizeState::Done => {
            for raw_line in plugin.summary_text.lines() {
                lines.push(Line::from(Span::styled(raw_line.to_string(), text_style)));
            }
            if plugin.summary_text.ends_with('\n') || plugin.summary_text.is_empty() {
                lines.push(Line::from(""));
            }
            if state == SummarizeState::Streaming {
                lines.push(Line::from(Span::styled(
                    format!("{spinner}"),
                    hint_style,
                )));
            }
        }
        SummarizeState::Error => {
            let msg = plugin.error.as_deref().unwrap_or("Unknown error");
            lines.push(Line::from(Span::styled(msg.to_string(), error_style)));
        }
        SummarizeState::Idle => {}
    }

    // Apply scroll offset
    let total_lines = lines.len();
    let scroll = plugin.scroll_offset.min(total_lines.saturating_sub(content_height));
    let visible: Vec<Line> = lines.into_iter().skip(scroll).take(content_height).collect();

    let hint = match state {
        SummarizeState::Done | SummarizeState::Error => "j/k: scroll  q/Esc: close",
        _ => "q/Esc: cancel",
    };
    let hint_line = Line::from(Span::styled(hint.to_string(), hint_style));

    // Build content: visible lines + spacer + hint
    let mut content_lines = visible;
    // Pad to fill remaining space so hint is at bottom
    while content_lines.len() < content_height {
        content_lines.push(Line::from(""));
    }
    content_lines.push(hint_line);

    frame.render_widget(Clear, popup);
    let paragraph = Paragraph::new(Text::from(content_lines))
        .wrap(Wrap { trim: false })
        .block(block)
        .style(Style::default().bg(theme::palette().surface2));
    frame.render_widget(paragraph, popup);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x.saturating_add(area.width.saturating_sub(width) / 2);
    let y = area.y.saturating_add(area.height.saturating_sub(height) / 2);
    Rect { x, y, width, height }
}
