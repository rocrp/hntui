use crate::plugin::summarize::{SummarizePlugin, SummarizeState};
use crate::ui::{markdown, theme};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

pub fn render(frame: &mut Frame, plugin: &SummarizePlugin, spinner: char) {
    if !plugin.is_overlay_visible() {
        return;
    }

    let area = frame.area();
    let Some(popup) = popup_rect(area) else {
        return;
    };

    let state = plugin.state();

    let model_tag = if plugin.model_name.is_empty() {
        String::new()
    } else {
        format!(" ({})", plugin.model_name)
    };

    let title = match state {
        SummarizeState::Loading => {
            if plugin.reasoning_buffer.is_empty() {
                format!(
                    " Summarizing {spinner} ({} comments){model_tag} ",
                    plugin.comment_count
                )
            } else {
                format!(" Thinking {spinner}{model_tag} ")
            }
        }
        SummarizeState::Streaming => {
            if plugin.summary_text.is_empty() {
                format!(" Thinking {spinner}{model_tag} ")
            } else {
                format!(" Summarizing {spinner}{model_tag} ")
            }
        }
        SummarizeState::Done => format!(" Summary{model_tag} "),
        SummarizeState::Error => " Summary Error ".to_string(),
        SummarizeState::Idle => return,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, theme::HEADER_ACCENT));
    let inner = block.inner(popup);

    let layout = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(inner);
    let content_area = layout[0];
    let hint_area = layout[1];

    let lines: Vec<Line> = match state {
        SummarizeState::Loading => {
            if plugin.reasoning_buffer.is_empty() {
                vec![Line::from(Span::styled(
                    format!("Waiting for LLM response {spinner}"),
                    theme::HINT,
                ))]
            } else {
                reasoning_lines(&plugin.reasoning_buffer, spinner, true)
            }
        }
        SummarizeState::Streaming => {
            if plugin.summary_text.is_empty() {
                reasoning_lines(&plugin.reasoning_buffer, spinner, true)
            } else {
                let mut l = markdown::render_markdown(&plugin.summary_text);
                l.push(Line::from(Span::styled(format!("{spinner}"), theme::HINT)));
                l
            }
        }
        SummarizeState::Done => markdown::render_markdown(&plugin.summary_text),
        SummarizeState::Error => {
            let msg = plugin.error.as_deref().unwrap_or("Unknown error");
            vec![Line::from(Span::styled(msg.to_string(), theme::ERROR))]
        }
        SummarizeState::Idle => vec![],
    };

    frame.render_widget(Clear, popup);
    frame.render_widget(block.style(theme::POPUP), popup);

    let content_paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((plugin.scroll_offset as u16, 0))
        .style(theme::POPUP);
    frame.render_widget(content_paragraph, content_area);

    let show_copied = plugin
        .copied_flash
        .is_some_and(|t| t.elapsed().as_secs() < 2);
    let hint_line = if show_copied {
        Line::from(Span::styled("Copied!", theme::SUCCESS))
    } else {
        let hint = match state {
            SummarizeState::Done => "j/k: scroll  c: copy  q/Esc: close",
            SummarizeState::Streaming => "j/k: scroll  c: copy  q/Esc: cancel",
            SummarizeState::Error => "j/k: scroll  q/Esc: close",
            _ => "q/Esc: cancel",
        };
        Line::from(Span::styled(hint, theme::HINT))
    };
    let hint_paragraph = Paragraph::new(hint_line).style(theme::POPUP);
    frame.render_widget(hint_paragraph, hint_area);
}

pub(crate) fn popup_rect(area: Rect) -> Option<Rect> {
    if area.width < 12 || area.height < 8 {
        return None;
    }
    let popup_w = (area.width * 4 / 5).max(30);
    let popup_h = (area.height * 4 / 5).max(10);
    Some(super::centered(area, popup_w, popup_h))
}

pub(crate) fn content_height(area: Rect) -> Option<usize> {
    let popup = popup_rect(area)?;
    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(popup);
    let layout = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(inner);
    Some(layout[0].height as usize)
}

fn reasoning_lines(buffer: &str, spinner: char, streaming: bool) -> Vec<Line<'static>> {
    use ratatui::style::{Modifier, Style};

    let dim_style = Style::default()
        .fg(theme::OVERLAY0)
        .add_modifier(Modifier::DIM | Modifier::ITALIC);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let label = if streaming {
        format!("Thinking {spinner}")
    } else {
        "Thinking".to_string()
    };
    lines.push(Line::from(Span::styled(label, theme::HINT)));
    lines.push(Line::raw(""));

    for raw in buffer.lines() {
        lines.push(Line::from(Span::styled(raw.to_string(), dim_style)));
    }
    lines
}
