use crate::app::App;
use crate::ui::theme;
use crate::ui::{format_age, now_unix};
use html_escape::decode_html_entities;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let spinner = app.spinner_frame();
    let title = app
        .current_story
        .as_ref()
        .map(|s| s.title.as_str())
        .unwrap_or("Comments");
    let title = if app.comment_loading {
        format!("{title} (loading {spinner})")
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

    let layout = theme::layout();
    let comment_max_lines = layout.comment_max_lines.unwrap_or(usize::MAX);
    let content_width = list_area.width as usize;

    let highlight_style = Style::default()
        .bg(theme::palette().surface2)
        .add_modifier(Modifier::BOLD);

    if app.comment_loading && app.comment_list.is_empty() {
        app.comment_item_heights.clear();
        app.comment_line_offset = 0;
        app.comment_viewport_height = list_area.height as usize;
        app.comment_page_size = app.comment_viewport_height.max(1);

        let items = vec![ListItem::new(Line::from(format!("Loading {spinner}")))];
        let list = List::new(items)
            .highlight_symbol("")
            .highlight_style(highlight_style);
        frame.render_stateful_widget(list, list_area, &mut app.comment_list_state);
    } else if app.comment_list.is_empty() {
        app.comment_item_heights.clear();
        app.comment_line_offset = 0;
        app.comment_viewport_height = list_area.height as usize;
        app.comment_page_size = app.comment_viewport_height.max(1);

        let items = vec![ListItem::new(Line::from("No comments."))];
        let list = List::new(items)
            .highlight_symbol("")
            .highlight_style(highlight_style);
        frame.render_stateful_widget(list, list_area, &mut app.comment_list_state);
    } else {
        let now = now_unix();
        let mut comment_lines = Vec::with_capacity(app.comment_list.len());

        for comment in &app.comment_list {
            let indent = "│ ".repeat(comment.depth);
            let indent_width = indent.chars().count();
            let indent_style = Style::default().fg(theme::comment_indent_color(comment.depth));
            let marker_style = indent_style.add_modifier(Modifier::BOLD);

            let thread_marker = if comment.kids.is_empty() {
                ' '
            } else if comment.collapsed {
                '▸'
            } else if comment.children_loading {
                spinner
            } else {
                '▾'
            };

            let by = comment
                .by
                .as_deref()
                .unwrap_or(if comment.deleted {
                    "[deleted]"
                } else {
                    "[unknown]"
                })
                .to_string();
            let age = comment
                .time
                .map(|t| format_age(t, now))
                .unwrap_or_else(|| "?".to_string());

            let author_style = Style::default()
                .fg(theme::palette().subtext0)
                .add_modifier(Modifier::ITALIC);
            let content_style = Style::default().fg(theme::palette().text);
            let tail_style = Style::default().fg(theme::palette().overlay0);

            let tail = if comment.dead && !comment.deleted {
                format!(" [dead] | {age}")
            } else {
                format!(" | {age}")
            };
            let tail_width = tail.chars().count();

            let prefix_width = indent_width + 2 + by.chars().count() + 2;
            let first_width = content_width
                .saturating_sub(prefix_width)
                .saturating_sub(tail_width);
            let next_width = content_width
                .saturating_sub(indent_width)
                .saturating_sub(2)
                .max(1);

            let plain = hn_html_to_plain(&comment.text);
            let wrapped = wrap_plain(&plain, first_width.max(1), next_width, comment_max_lines);
            let header_content = wrapped.first().cloned().unwrap_or_default();

            let mut lines = Vec::with_capacity(wrapped.len());
            lines.push(Line::from(vec![
                Span::styled(indent.clone(), indent_style),
                Span::styled(format!("{thread_marker} "), marker_style),
                Span::styled(by, author_style),
                Span::raw(": "),
                Span::styled(header_content, content_style),
                Span::styled(tail, tail_style),
            ]));

            for line in wrapped.into_iter().skip(1) {
                lines.push(Line::from(vec![
                    Span::styled(indent.clone(), indent_style),
                    Span::raw("  "),
                    Span::styled(line, content_style),
                ]));
            }

            if lines.is_empty() {
                lines.push(Line::from(""));
            }

            comment_lines.push(lines);
        }

        app.comment_item_heights = comment_lines
            .iter()
            .map(|lines| lines.len().max(1))
            .collect();

        app.comment_viewport_height = list_area.height as usize;
        let total_lines: usize = app.comment_item_heights.iter().sum();
        let avg_height = if app.comment_item_heights.is_empty() {
            1
        } else {
            (total_lines / app.comment_item_heights.len()).max(1)
        };
        let viewport_height = app.comment_viewport_height.max(1);
        app.comment_page_size = (viewport_height / avg_height).max(1);

        if app.comment_list_state.selected().is_none() {
            app.comment_list_state.select(Some(0));
        }
        app.ensure_comment_line_offset();

        let max_offset = total_lines.saturating_sub(viewport_height);
        app.comment_line_offset = app.comment_line_offset.min(max_offset);
        let start = app.comment_line_offset;
        let end = (start + viewport_height).min(total_lines);

        let selected = app.comment_list_state.selected().unwrap_or(0);
        let mut visible_lines = Vec::with_capacity(end.saturating_sub(start));
        let mut line_idx = 0usize;
        'outer: for (idx, lines) in comment_lines.iter().enumerate() {
            for line in lines {
                if line_idx >= start && line_idx < end {
                    let mut line = line.clone();
                    if idx == selected {
                        line = line.patch_style(highlight_style);
                    }
                    visible_lines.push(line);
                }
                line_idx += 1;
                if line_idx >= end {
                    break 'outer;
                }
            }
        }

        if visible_lines.is_empty() {
            visible_lines.push(Line::from(""));
        }

        frame.render_widget(Paragraph::new(visible_lines), list_area);
    }

    let footer_block = Block::default().borders(Borders::TOP);
    let footer_inner = footer_block.inner(footer_area);
    frame.render_widget(footer_block, footer_area);

    let now = now_unix();
    let meta = if let Some(err) = app.last_error.as_deref() {
        Line::from(vec![Span::styled(
            format!("Error: {err}"),
            Style::default().fg(theme::palette().red),
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
        "j/k:nav  h/←:collapse  l/→:expand  Enter/c:toggle  o:comments  O:source  r:refresh  ?:help  q:back    {} comments",
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

fn wrap_plain(s: &str, first_width: usize, next_width: usize, max_lines: usize) -> Vec<String> {
    if max_lines == 0 {
        return vec![String::new()];
    }

    let mut out = Vec::new();
    let mut current = String::new();

    for raw_line in s.split('\n') {
        let line = collapse_spaces(raw_line.trim());
        if line.is_empty() {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
                if out.len() >= max_lines {
                    return out;
                }
            }
            continue;
        }

        for word in line.split_whitespace() {
            let width = if out.is_empty() {
                first_width
            } else {
                next_width
            };
            let width = width.max(1);

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

            out.push(std::mem::take(&mut current));
            if out.len() >= max_lines {
                return out;
            }
            current.push_str(word);
        }

        if !current.is_empty() {
            out.push(std::mem::take(&mut current));
            if out.len() >= max_lines {
                return out;
            }
        }
    }

    if out.is_empty() {
        out.push(String::new());
    }
    out
}
