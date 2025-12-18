use crate::app::App;
use crate::ui::{format_age, now_unix};
use crate::ui::theme;
use html_escape::decode_html_entities;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let title = app
        .current_story
        .as_ref()
        .map(|s| s.title.as_str())
        .unwrap_or("Comments");
    let title = if app.comment_loading {
        format!("{title} (loading)")
    } else {
        title.to_string()
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [list_area, footer_area] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .areas(inner);

    let comment_item_height = 3;
    app.comment_page_size = (list_area.height as usize)
        .saturating_div(comment_item_height)
        .max(1);
    let content_width = list_area.width.saturating_sub(2) as usize;

    let items = if app.comment_loading && app.comment_list.is_empty() {
        vec![ListItem::new(Line::from("Loading…"))]
    } else if app.comment_list.is_empty() {
        vec![ListItem::new(Line::from("No comments."))]
    } else {
        let now = now_unix();
        app.comment_list
            .iter()
            .map(|comment| {
                let indent = "│ ".repeat(comment.depth);
                let indent_width = indent.chars().count();
                let indent_style = Style::default().fg(theme::rainbow_depth(comment.depth));
                let marker_style = indent_style.add_modifier(Modifier::BOLD);

                let thread_marker = if comment.kids.is_empty() {
                    ' '
                } else if comment.collapsed {
                    '▸'
                } else {
                    '▾'
                };

                let by = comment.by.as_deref().unwrap_or(if comment.deleted {
                    "[deleted]"
                } else {
                    "[unknown]"
                });
                let age = comment
                    .time
                    .map(|t| format_age(t, now))
                    .unwrap_or_else(|| "?".to_string());

                let mut header_spans = vec![
                    Span::styled(indent.clone(), indent_style),
                    Span::styled(format!("{thread_marker} "), marker_style),
                    Span::styled(
                        by,
                        Style::default()
                            .fg(theme::SUBTEXT0)
                            .add_modifier(Modifier::ITALIC),
                    ),
                    Span::styled(format!(" ({age})"), Style::default().fg(theme::OVERLAY0)),
                ];
                if comment.dead && !comment.deleted {
                    header_spans.push(Span::styled(
                        " [dead]",
                        Style::default().fg(theme::OVERLAY0),
                    ));
                }
                let header = Line::from(header_spans);

                let plain = hn_html_to_plain(&comment.text);
                let wrap_width = content_width
                    .saturating_sub(indent_width)
                    .saturating_sub(2)
                    .max(1);
                let mut body_lines = wrap_plain(&plain, wrap_width);
                body_lines.truncate(comment_item_height - 1);
                while body_lines.len() < comment_item_height - 1 {
                    body_lines.push(String::new());
                }

                let mut lines = Vec::with_capacity(comment_item_height);
                lines.push(header);
                for line in body_lines {
                    lines.push(Line::from(vec![
                        Span::styled(indent.clone(), indent_style),
                        Span::raw("  "),
                        Span::raw(line),
                    ]));
                }

                ListItem::new(Text::from(lines))
            })
            .collect::<Vec<_>>()
    };

    let list = List::new(items)
        .highlight_symbol("▶ ")
        .repeat_highlight_symbol(false)
        .highlight_style(
            Style::default()
                .bg(theme::SURFACE0)
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_stateful_widget(list, list_area, &mut app.comment_list_state);

    let footer_block = Block::default().borders(Borders::TOP);
    let footer_inner = footer_block.inner(footer_area);
    frame.render_widget(footer_block, footer_area);

    let now = now_unix();
    let meta = if let Some(err) = app.last_error.as_deref() {
        Line::from(vec![Span::styled(
            format!("Error: {err}"),
            Style::default().fg(Color::Red),
        )])
    } else if let Some(story) = app.current_story.as_ref() {
        let age = format_age(story.time, now);
        Line::from(format!(
            "{} pts by {} {age} | {} comments",
            story.score, story.by, story.comment_count
        ))
    } else if app.comment_loading {
        Line::from("Loading…")
    } else {
        Line::from("")
    };

    let help = Line::from(format!(
        "j/k:nav  c:collapse  o:open  r:refresh  q:back    {} comments",
        app.comment_list.len()
    ));
    frame.render_widget(Paragraph::new(vec![meta, help]), footer_inner);
}

fn hn_html_to_plain(html: &str) -> String {
    let html = html
        .replace("<p>", "\n\n")
        .replace("</p>", "\n\n")
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n");

    let mut stripped = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => stripped.push(ch),
            _ => {}
        }
    }

    let decoded = decode_html_entities(&stripped).into_owned();
    decoded
        .lines()
        .map(|line| collapse_spaces(line.trim()))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn collapse_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            prev_space = false;
            out.push(ch);
        }
    }
    out
}

fn wrap_plain(s: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }

    let mut out = Vec::new();
    for line in s.split('\n') {
        let mut current = String::new();
        for word in line.split_whitespace() {
            if current.is_empty() {
                current.push_str(word);
                continue;
            }

            let next_len = current.chars().count() + 1 + word.chars().count();
            if next_len <= width {
                current.push(' ');
                current.push_str(word);
                continue;
            }

            out.push(current);
            current = word.to_string();
        }
        if !current.is_empty() {
            out.push(current);
        }
        if out.is_empty() {
            out.push(String::new());
        }
    }
    out
}
