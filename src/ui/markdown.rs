use crate::ui::theme;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

pub fn render_markdown(input: &str) -> Vec<Line<'static>> {
    let opts = Options::ENABLE_STRIKETHROUGH;
    let parser = Parser::new_ext(input, opts);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut style_stack: Vec<Style> = Vec::new();
    let mut prefix_spans: Vec<Span<'static>> = Vec::new();
    let mut list_depth: usize = 0;
    let mut list_index_stack: Vec<Option<u64>> = Vec::new();
    let mut in_code_block = false;
    let mut need_paragraph_break = false;

    let base_style = Style::default().fg(theme::palette().text);

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    flush_line(&mut lines, &mut current_spans, &prefix_spans);
                    if need_paragraph_break {
                        lines.push(Line::from(""));
                        need_paragraph_break = false;
                    }
                    let mut style = Style::default()
                        .fg(theme::palette().mauve)
                        .add_modifier(Modifier::BOLD);
                    if level == pulldown_cmark::HeadingLevel::H1 {
                        style = style.add_modifier(Modifier::UNDERLINED);
                    }
                    style_stack.push(style);
                }
                Tag::Paragraph => {
                    if need_paragraph_break && !in_code_block {
                        flush_line(&mut lines, &mut current_spans, &prefix_spans);
                        lines.push(Line::from(""));
                    }
                    need_paragraph_break = false;
                }
                Tag::Strong => {
                    let top = current_style(&style_stack, base_style);
                    style_stack.push(top.add_modifier(Modifier::BOLD));
                }
                Tag::Emphasis => {
                    let top = current_style(&style_stack, base_style);
                    style_stack.push(top.add_modifier(Modifier::ITALIC));
                }
                Tag::Strikethrough => {
                    let top = current_style(&style_stack, base_style);
                    style_stack.push(top.add_modifier(Modifier::CROSSED_OUT));
                }
                Tag::CodeBlock(_) => {
                    flush_line(&mut lines, &mut current_spans, &prefix_spans);
                    if need_paragraph_break {
                        lines.push(Line::from(""));
                    }
                    in_code_block = true;
                    need_paragraph_break = false;
                }
                Tag::List(start) => {
                    if list_depth == 0 {
                        flush_line(&mut lines, &mut current_spans, &prefix_spans);
                        if need_paragraph_break {
                            lines.push(Line::from(""));
                            need_paragraph_break = false;
                        }
                    }
                    list_index_stack.push(start);
                    list_depth += 1;
                }
                Tag::Item => {
                    flush_line(&mut lines, &mut current_spans, &prefix_spans);
                    let indent = "  ".repeat(list_depth.saturating_sub(1));
                    let marker = match list_index_stack.last_mut() {
                        Some(Some(idx)) => {
                            let m = format!("{indent}{idx}. ");
                            *idx += 1;
                            m
                        }
                        _ => format!("{indent}- "),
                    };
                    prefix_spans = vec![Span::styled(
                        marker,
                        Style::default().fg(theme::palette().blue),
                    )];
                }
                Tag::BlockQuote(_) => {
                    flush_line(&mut lines, &mut current_spans, &prefix_spans);
                    prefix_spans = vec![Span::styled(
                        "> ".to_string(),
                        Style::default().fg(theme::palette().green),
                    )];
                }
                Tag::Link { dest_url, .. } => {
                    let top = current_style(&style_stack, base_style);
                    let link_style = top
                        .fg(theme::palette().blue)
                        .add_modifier(Modifier::UNDERLINED);
                    style_stack.push(link_style);
                    // Store URL for appending after link text
                    style_stack.push(Style::default()); // sentinel
                                                        // We'll handle the URL in End
                    let _ = dest_url; // used in TagEnd
                }
                _ => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Heading(_) => {
                    flush_line(&mut lines, &mut current_spans, &prefix_spans);
                    style_stack.pop();
                    need_paragraph_break = true;
                }
                TagEnd::Paragraph => {
                    flush_line(&mut lines, &mut current_spans, &prefix_spans);
                    need_paragraph_break = true;
                }
                TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough => {
                    style_stack.pop();
                }
                TagEnd::CodeBlock => {
                    in_code_block = false;
                    need_paragraph_break = true;
                }
                TagEnd::List(_) => {
                    list_depth = list_depth.saturating_sub(1);
                    list_index_stack.pop();
                    if list_depth == 0 {
                        need_paragraph_break = true;
                    }
                }
                TagEnd::Item => {
                    flush_line(&mut lines, &mut current_spans, &prefix_spans);
                    prefix_spans.clear();
                }
                TagEnd::BlockQuote(_) => {
                    flush_line(&mut lines, &mut current_spans, &prefix_spans);
                    prefix_spans.clear();
                    need_paragraph_break = true;
                }
                TagEnd::Link => {
                    style_stack.pop(); // sentinel
                    style_stack.pop(); // link style
                }
                _ => {}
            },
            Event::Text(text) => {
                if in_code_block {
                    let code_style = Style::default().fg(theme::palette().teal);
                    for line_str in text.split('\n') {
                        if !current_spans.is_empty() {
                            flush_line(&mut lines, &mut current_spans, &prefix_spans);
                        }
                        current_spans.push(Span::styled(format!("  {line_str}"), code_style));
                    }
                } else {
                    let style = current_style(&style_stack, base_style);
                    // Handle text with newlines for soft breaks in headings etc.
                    let parts: Vec<&str> = text.split('\n').collect();
                    for (i, part) in parts.iter().enumerate() {
                        if i > 0 {
                            flush_line(&mut lines, &mut current_spans, &prefix_spans);
                        }
                        if !part.is_empty() {
                            current_spans.push(Span::styled(part.to_string(), style));
                        }
                    }
                }
            }
            Event::Code(code) => {
                let style = Style::default().fg(theme::palette().teal);
                current_spans.push(Span::styled(format!("`{code}`"), style));
            }
            Event::SoftBreak => {
                current_spans.push(Span::raw(" "));
            }
            Event::HardBreak => {
                flush_line(&mut lines, &mut current_spans, &prefix_spans);
            }
            Event::Rule => {
                flush_line(&mut lines, &mut current_spans, &prefix_spans);
                lines.push(Line::from(Span::styled(
                    "───────────────────────",
                    Style::default().fg(theme::palette().overlay0),
                )));
                need_paragraph_break = true;
            }
            _ => {}
        }
    }

    flush_line(&mut lines, &mut current_spans, &prefix_spans);
    lines
}

fn current_style(stack: &[Style], base: Style) -> Style {
    stack.last().copied().unwrap_or(base)
}

fn flush_line(
    lines: &mut Vec<Line<'static>>,
    current_spans: &mut Vec<Span<'static>>,
    prefix_spans: &[Span<'static>],
) {
    if current_spans.is_empty() {
        return;
    }
    let mut spans = prefix_spans.to_vec();
    spans.append(current_spans);
    lines.push(Line::from(spans));
}
