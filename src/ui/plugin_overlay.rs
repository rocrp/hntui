use crate::plugin::summarize::{SummarizePlugin, SummarizeState};
use crate::ui::{markdown, theme};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

pub fn render(frame: &mut Frame, plugin: &mut SummarizePlugin, spinner: char) {
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
    let hint_style = Style::default().fg(theme::palette().subtext0);
    let error_style = Style::default().fg(theme::palette().red);
    let bg = Style::default().bg(theme::palette().surface2);

    let state = plugin.state();

    let model_tag = if plugin.model_name.is_empty() {
        String::new()
    } else {
        format!(" ({})", plugin.model_name)
    };

    let title = match state {
        SummarizeState::Loading => {
            format!(
                " Summarizing {spinner} ({} comments){model_tag} ",
                plugin.comment_count
            )
        }
        SummarizeState::Streaming => format!(" Summarizing {spinner}{model_tag} "),
        SummarizeState::Done => format!(" Summary{model_tag} "),
        SummarizeState::Error => " Summary Error ".to_string(),
        SummarizeState::Idle => return,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, header_style));
    let inner = block.inner(popup);

    // Split inner into [content, hint(1 line)]
    let layout = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(inner);
    let content_area = layout[0];
    let hint_area = layout[1];

    // Store content height for page scroll calculations
    plugin.content_height = content_area.height as usize;

    // Build content lines
    let lines: Vec<Line> = match state {
        SummarizeState::Loading => {
            vec![Line::from(Span::styled(
                format!("Waiting for LLM response {spinner}"),
                hint_style,
            ))]
        }
        SummarizeState::Streaming | SummarizeState::Done => {
            let mut l = markdown::render_markdown(&plugin.summary_text);
            if state == SummarizeState::Streaming {
                l.push(Line::from(Span::styled(format!("{spinner}"), hint_style)));
            }
            l
        }
        SummarizeState::Error => {
            let msg = plugin.error.as_deref().unwrap_or("Unknown error");
            vec![Line::from(Span::styled(msg.to_string(), error_style))]
        }
        SummarizeState::Idle => vec![],
    };

    // Clear popup area and fill background
    frame.render_widget(Clear, popup);

    // Render block (border + title + background) on full popup rect
    frame.render_widget(block.style(bg), popup);

    // Render content paragraph (without block) into content_area only,
    // so scrolled text never bleeds into the hint row
    let content_paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((plugin.scroll_offset as u16, 0))
        .style(bg);
    frame.render_widget(content_paragraph, content_area);

    // Render hint at bottom (not affected by scroll)
    let hint = match state {
        SummarizeState::Done | SummarizeState::Error => "j/k: scroll  q/Esc: close",
        _ => "q/Esc: cancel",
    };
    let hint_paragraph = Paragraph::new(Line::from(Span::styled(hint, hint_style))).style(bg);
    frame.render_widget(hint_paragraph, hint_area);
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
