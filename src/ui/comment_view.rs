use crate::app::{App, LayoutAreas};
use crate::ui::theme;
use crate::ui::{format_age, format_error, now_unix};
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

    app.layout_areas = LayoutAreas {
        list_area,
        frame_area: area,
    };
    let comment_max_lines = theme::COMMENT_MAX_LINES.unwrap_or(usize::MAX);
    let content_width = list_area.width as usize;

    if app.comment_loading && app.comment_list.is_empty() {
        app.comment_item_heights.clear();
        app.comment_line_offset = 0;
        app.comment_viewport_height = list_area.height as usize;
        app.comment_page_size = app.comment_viewport_height.max(1);

        let items = vec![ListItem::new(Line::from(format!("Loading {spinner}")))];
        let list = List::new(items)
            .highlight_symbol("")
            .highlight_style(theme::SELECTED);
        frame.render_stateful_widget(list, list_area, &mut app.comment_list_state);
    } else if app.comment_list.is_empty() {
        app.comment_item_heights.clear();
        app.comment_line_offset = 0;
        app.comment_viewport_height = list_area.height as usize;
        app.comment_page_size = app.comment_viewport_height.max(1);

        let items = vec![ListItem::new(Line::from("No comments."))];
        let list = List::new(items)
            .highlight_symbol("")
            .highlight_style(theme::SELECTED);
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
                .fg(theme::comment_indent_color(comment.depth))
                .add_modifier(Modifier::BOLD);

            let header_spans = vec![
                Span::styled(indent.clone(), indent_style),
                Span::styled(format!("{thread_marker} "), marker_style),
                Span::styled(by, author_style),
                Span::styled(format!(" · {age}"), theme::META),
            ];

            let mut lines = Vec::new();
            lines.push(Line::from(header_spans));

            let body_indent = format!("{indent}  ");
            let body_width = content_width.saturating_sub(indent_width + 2).max(1);
            let plain = hn_html_to_plain(&comment.text);
            let content_style = Style::default().fg(theme::TEXT);

            if !plain.is_empty() {
                let wrapped = wrap_content(&plain, body_width, comment_max_lines);
                for wline in wrapped {
                    match wline {
                        ContentLine::Normal(text) => {
                            lines.push(Line::from(vec![
                                Span::styled(body_indent.clone(), indent_style),
                                Span::styled(text, content_style),
                            ]));
                        }
                        ContentLine::Quote(text) => {
                            lines.push(Line::from(vec![
                                Span::styled(body_indent.clone(), indent_style),
                                Span::styled("▎ ", theme::QUOTE_BAR),
                                Span::styled(text, theme::QUOTE),
                            ]));
                        }
                        ContentLine::Blank => {
                            lines.push(Line::from(Span::styled(body_indent.clone(), indent_style)));
                        }
                    }
                }
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

        let mut line_starts = Vec::with_capacity(app.comment_item_heights.len() + 1);
        line_starts.push(0usize);
        let mut cumsum = 0usize;
        for &h in &app.comment_item_heights {
            cumsum += h;
            line_starts.push(cumsum);
        }

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
        let sel_start = line_starts.get(selected).copied().unwrap_or(0);
        let sel_end = line_starts.get(selected + 1).copied().unwrap_or(sel_start);
        let half_viewport = viewport_height / 2;

        let mut visible_lines = Vec::with_capacity(end.saturating_sub(start));
        let mut line_idx = 0usize;
        let dim_target = theme::OVERLAY0;
        'outer: for (idx, lines) in comment_lines.iter().enumerate() {
            for (line_in_comment, line) in lines.iter().enumerate() {
                if line_idx >= start && line_idx < end {
                    let mut line = line.clone();
                    let dist = if line_idx < sel_start {
                        sel_start - line_idx
                    } else if line_idx >= sel_end {
                        line_idx - sel_end + 1
                    } else {
                        0
                    };
                    if dist > 0 {
                        let max_dist = half_viewport.max(1) as f64;
                        let fade = (dist as f64 / max_dist).min(1.0);
                        let dim_factor = fade * 0.7;
                        line = Line::from(
                            line.spans
                                .into_iter()
                                .map(|span| {
                                    if let Some(fg) = span.style.fg {
                                        Span::styled(
                                            span.content,
                                            span.style.fg(theme::blend(fg, dim_target, dim_factor)),
                                        )
                                    } else {
                                        span
                                    }
                                })
                                .collect::<Vec<_>>(),
                        );
                    }

                    if idx == selected && line_in_comment == 0 {
                        line = line.patch_style(theme::SELECTED);
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
    let show_copied = app.copied_flash.is_some_and(|t| t.elapsed().as_secs() < 2);
    let meta = if show_copied {
        Line::from(Span::styled("Copied!", theme::SUCCESS))
    } else if let Some(err) = app.last_error.as_deref() {
        Line::from(vec![Span::styled(
            format!("Error: {}", format_error(err)),
            theme::ERROR,
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
        "j/k:nav  h/←:collapse  l/→:expand  Enter/c:toggle  y:copy  s:summarize  o:comments  O:source  r:refresh  ?:help  q:back    {} comments",
        app.comment_list.len()
    ));
    frame.render_widget(Paragraph::new(vec![meta, help]), footer_inner);
}

pub(crate) fn hn_html_to_plain(html: &str) -> String {
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

    let mut result = Vec::new();
    let mut prev_empty = false;
    for line in decoded.lines() {
        let trimmed = collapse_spaces(line.trim());
        if trimmed.is_empty() {
            if !prev_empty && !result.is_empty() {
                result.push(String::new());
                prev_empty = true;
            }
        } else {
            result.push(trimmed);
            prev_empty = false;
        }
    }
    while result.last().is_some_and(|s| s.is_empty()) {
        result.pop();
    }
    result.join("\n")
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

enum ContentLine {
    Normal(String),
    Quote(String),
    Blank,
}

fn wrap_content(s: &str, width: usize, max_lines: usize) -> Vec<ContentLine> {
    if max_lines == 0 {
        return vec![ContentLine::Normal(String::new())];
    }
    let width = width.max(1);
    let quote_width = width.saturating_sub(2).max(1);

    let mut paragraphs: Vec<(String, bool)> = Vec::new();
    let mut current_para = String::new();
    let mut is_quote = false;
    let mut first_line = true;

    for raw_line in s.split('\n') {
        let line = collapse_spaces(raw_line.trim());
        if line.is_empty() {
            if !current_para.is_empty() {
                paragraphs.push((std::mem::take(&mut current_para), is_quote));
                first_line = true;
            }
            continue;
        }

        if first_line {
            is_quote = line.starts_with('>');
            first_line = false;
        }

        let text = if is_quote {
            line.trim_start_matches('>').trim_start().to_string()
        } else {
            line
        };

        if !current_para.is_empty() {
            current_para.push(' ');
        }
        current_para.push_str(&text);
    }
    if !current_para.is_empty() {
        paragraphs.push((current_para, is_quote));
    }

    let mut out = Vec::new();
    for (pi, (para, quoted)) in paragraphs.iter().enumerate() {
        if pi > 0 {
            out.push(ContentLine::Blank);
            if out.len() >= max_lines {
                return out;
            }
        }

        let w = if *quoted { quote_width } else { width };
        let mut current = String::new();
        for word in para.split_whitespace() {
            if current.is_empty() {
                current.push_str(word);
                continue;
            }
            let next_len = current.chars().count() + 1 + word.chars().count();
            if next_len <= w {
                current.push(' ');
                current.push_str(word);
                continue;
            }
            let line = std::mem::take(&mut current);
            out.push(if *quoted {
                ContentLine::Quote(line)
            } else {
                ContentLine::Normal(line)
            });
            if out.len() >= max_lines {
                return out;
            }
            current.push_str(word);
        }
        if !current.is_empty() {
            out.push(if *quoted {
                ContentLine::Quote(current)
            } else {
                ContentLine::Normal(current)
            });
            if out.len() >= max_lines {
                return out;
            }
        }
    }

    if out.is_empty() {
        out.push(ContentLine::Normal(String::new()));
    }
    out
}
