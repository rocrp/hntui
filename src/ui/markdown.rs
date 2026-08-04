use std::ops::RangeInclusive;

use crate::ui::theme;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};
use unicode_width::UnicodeWidthStr;
use url::{ParseError, Url};

const LINK_PROBE_BACKGROUND: Color = Color::Rgb(1, 2, 3);
const LINK_PROBE_STYLE: Style = Style::new().bg(LINK_PROBE_BACKGROUND);

pub fn render_markdown(input: &str) -> Vec<Line<'static>> {
    render_markdown_document(input, None, None).lines
}

pub struct MarkdownDocument {
    pub lines: Vec<Line<'static>>,
    pub links: Vec<String>,
}

pub fn render_markdown_document(
    input: &str,
    base_url: Option<&str>,
    selected_link: Option<usize>,
) -> MarkdownDocument {
    render_markdown_document_with_style(
        input,
        base_url,
        selected_link,
        theme::ARTICLE_LINK_SELECTED,
    )
}

fn render_markdown_document_with_style(
    input: &str,
    base_url: Option<&str>,
    selected_link: Option<usize>,
    selected_link_style: Style,
) -> MarkdownDocument {
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
    let mut links = Vec::new();
    let mut active_link = None;

    let base_style = Style::default().fg(theme::TEXT);

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    flush_line(&mut lines, &mut current_spans, &prefix_spans);
                    if need_paragraph_break {
                        lines.push(Line::from(""));
                        need_paragraph_break = false;
                    }
                    let mut style = theme::HEADER_ACCENT;
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
                    prefix_spans = vec![Span::styled(marker, theme::LIST_MARKER)];
                }
                Tag::BlockQuote(_) => {
                    flush_line(&mut lines, &mut current_spans, &prefix_spans);
                    prefix_spans = vec![Span::styled(
                        "> ".to_string(),
                        Style::default().fg(theme::GREEN),
                    )];
                }
                Tag::Link { dest_url, .. } => {
                    let top = current_style(&style_stack, base_style);
                    let link_index = resolve_article_link(&dest_url, base_url).map(|url| {
                        let index = links.len();
                        links.push(url);
                        index
                    });
                    let link_style = match link_index {
                        Some(index) if selected_link == Some(index) => selected_link_style,
                        Some(_) => top.fg(theme::BLUE).add_modifier(Modifier::UNDERLINED),
                        None => top,
                    };
                    active_link = link_index;
                    style_stack.push(link_style);
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
                    style_stack.pop();
                    active_link = None;
                }
                _ => {}
            },
            Event::Text(text) => {
                if in_code_block {
                    for line_str in text.lines() {
                        if !current_spans.is_empty() {
                            flush_line(&mut lines, &mut current_spans, &prefix_spans);
                        }
                        current_spans.push(Span::styled(format!("  {line_str}"), theme::CODE));
                    }
                } else {
                    let style = current_style(&style_stack, base_style);
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
                let style = if active_link.is_some() {
                    current_style(&style_stack, base_style)
                } else {
                    theme::CODE
                };
                current_spans.push(Span::styled(format!("`{code}`"), style));
            }
            Event::SoftBreak => {
                current_spans.push(Span::styled(" ", current_style(&style_stack, base_style)));
            }
            Event::HardBreak => {
                flush_line(&mut lines, &mut current_spans, &prefix_spans);
            }
            Event::Rule => {
                flush_line(&mut lines, &mut current_spans, &prefix_spans);
                lines.push(Line::from(Span::styled(
                    "───────────────────────",
                    theme::META,
                )));
                need_paragraph_break = true;
            }
            _ => {}
        }
    }

    flush_line(&mut lines, &mut current_spans, &prefix_spans);
    MarkdownDocument { lines, links }
}

fn resolve_article_link(destination: &str, base_url: Option<&str>) -> Option<String> {
    let url = match Url::parse(destination) {
        Ok(url) => url,
        Err(ParseError::RelativeUrlWithoutBase) => {
            Url::parse(base_url?).ok()?.join(destination).ok()?
        }
        Err(_) => return None,
    };
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    Some(url.into())
}

pub fn link_row_range(
    input: &str,
    base_url: Option<&str>,
    link_index: usize,
    width: u16,
) -> Option<RangeInclusive<usize>> {
    if width == 0 {
        return None;
    }
    let document =
        render_markdown_document_with_style(input, base_url, Some(link_index), LINK_PROBE_STYLE);
    document.links.get(link_index)?;
    let paragraph = Paragraph::new(document.lines).wrap(Wrap { trim: false });
    let height = u16::try_from(paragraph.line_count(width))
        .expect("article rendered height exceeds ratatui's u16 limit");
    if height == 0 {
        return None;
    }

    let area = Rect::new(0, 0, width, height);
    let mut buffer = Buffer::empty(area);
    paragraph.render(area, &mut buffer);

    let mut first = None;
    let mut last = None;
    for row in 0..height {
        let contains_link = (0..width).any(|column| {
            let cell = &buffer[(column, row)];
            cell.bg == LINK_PROBE_BACKGROUND
                && UnicodeWidthStr::width(cell.symbol()) > 0
                && cell
                    .symbol()
                    .chars()
                    .any(|character| !character.is_control())
        });
        if contains_link {
            first.get_or_insert(usize::from(row));
            last = Some(usize::from(row));
        }
    }
    Some(first?..=last.expect("first selected-link row has a last row"))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn line_texts(input: &str) -> Vec<String> {
        render_markdown(input)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.into_owned())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn renders_heading_and_paragraph_spacing() {
        let lines = line_texts("# Title\n\nBody **strong** and *em*");

        assert_eq!(lines, vec!["Title", "", "Body strong and em"]);
    }

    #[test]
    fn renders_nested_unordered_and_ordered_lists() {
        let lines = line_texts("- parent\n  - child\n\n3. third\n4. fourth");

        assert_eq!(
            lines,
            vec!["- parent", "  - child", "", "3. third", "4. fourth"]
        );
    }

    #[test]
    fn renders_block_quotes_and_code_blocks() {
        let lines = line_texts("> quoted\n\n```\nlet x = 1;\n```");

        assert_eq!(lines, vec!["> quoted", "", "  let x = 1;"]);
    }

    #[test]
    fn renders_links_as_underlined_text_without_url_suffix() {
        let lines = render_markdown("[site](https://example.com)");

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "site");
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::UNDERLINED));
    }
}
