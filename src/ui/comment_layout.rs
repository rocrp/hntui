use crate::api::types::Comment;
use crate::text::hn_html_to_plain;
use crate::ui::theme;
use crate::ui::{format_age, now_unix};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use std::ops::Range;

#[derive(Default)]
pub struct CommentLayout {
    comment_lines: Vec<Vec<Line<'static>>>,
    line_ranges: Vec<Range<usize>>,
    offset: usize,
    viewport_height: usize,
    width: usize,
}

impl CommentLayout {
    pub fn relayout(
        &mut self,
        comments: &[Comment],
        width: usize,
        viewport_height: usize,
        spinner: char,
    ) {
        self.width = width.max(1);
        self.viewport_height = viewport_height.max(1);
        self.comment_lines = build_comment_lines(comments, self.width, spinner);
        self.line_ranges.clear();

        let mut start = 0;
        for lines in &self.comment_lines {
            let end = start + lines.len().max(1);
            self.line_ranges.push(start..end);
            start = end;
        }
        assert_eq!(
            self.line_ranges.len(),
            comments.len(),
            "comment layout ranges out of sync"
        );
        self.offset = self.offset.min(self.max_offset());
    }

    pub fn invalidate(&mut self) {
        self.comment_lines.clear();
        self.line_ranges.clear();
    }

    pub fn line_range(&self, index: usize) -> Option<Range<usize>> {
        self.line_ranges.get(index).cloned()
    }

    pub fn hit_test(&self, viewport_row: usize) -> Option<usize> {
        if viewport_row >= self.viewport_height {
            return None;
        }
        let line = self.offset + viewport_row;
        let index = self.line_ranges.partition_point(|range| range.end <= line);
        self.line_ranges
            .get(index)
            .filter(|range| range.contains(&line))
            .map(|_| index)
    }

    pub fn ensure_visible(&mut self, index: usize) {
        if self.line_ranges.is_empty() {
            return;
        }
        let range = self
            .line_range(index)
            .unwrap_or_else(|| panic!("comment selection out of range: {index}"));
        let max_offset = self.max_offset();
        let height = range.end.saturating_sub(range.start);
        if height >= self.viewport_height {
            self.offset = range.start.min(max_offset);
            return;
        }

        self.offset = self.offset.min(max_offset);
        if range.start < self.offset {
            self.offset = range.start;
        } else if range.end > self.offset + self.viewport_height {
            self.offset = range.end.saturating_sub(self.viewport_height);
        }
        self.offset = self.offset.min(max_offset);
    }

    pub fn page_down(&mut self, selected: usize) -> usize {
        if self.line_ranges.is_empty() {
            self.offset = 0;
            return 0;
        }
        assert!(
            selected < self.line_ranges.len(),
            "comment selection out of range: {selected}"
        );
        let mut target = selected;
        let mut used = 0;
        while target + 1 < self.line_ranges.len() {
            let next = target + 1;
            let height = self.height(next);
            if used == 0 && height >= self.viewport_height {
                target = next;
                break;
            }
            if used + height > self.viewport_height {
                break;
            }
            used += height;
            target = next;
        }
        self.ensure_visible(target);
        target
    }

    pub fn page_up(&mut self, selected: usize) -> usize {
        if self.line_ranges.is_empty() {
            self.offset = 0;
            return 0;
        }
        assert!(
            selected < self.line_ranges.len(),
            "comment selection out of range: {selected}"
        );
        let mut target = selected;
        let mut used = 0;
        while target > 0 {
            let previous = target - 1;
            let height = self.height(previous);
            if used == 0 && height >= self.viewport_height {
                target = previous;
                break;
            }
            if used + height > self.viewport_height {
                break;
            }
            used += height;
            target = previous;
        }
        self.ensure_visible(target);
        target
    }

    pub fn visible_lines(&self, selected: usize) -> Vec<Line<'static>> {
        if self.line_ranges.is_empty() {
            return Vec::new();
        }
        let start = self.offset;
        let end = (start + self.viewport_height).min(self.total_lines());
        let selected_range = self
            .line_range(selected)
            .unwrap_or_else(|| panic!("comment selection out of range: {selected}"));
        let half_viewport = self.viewport_height / 2;
        let dim_target = theme::OVERLAY0;
        let mut visible = Vec::with_capacity(end.saturating_sub(start));

        for (index, lines) in self.comment_lines.iter().enumerate() {
            let range = &self.line_ranges[index];
            if range.end <= start {
                continue;
            }
            if range.start >= end {
                break;
            }
            for (line_in_comment, line) in lines.iter().enumerate() {
                let line_index = range.start + line_in_comment;
                if line_index < start || line_index >= end {
                    continue;
                }
                let mut line = line.clone();
                let distance = if line_index < selected_range.start {
                    selected_range.start - line_index
                } else if line_index >= selected_range.end {
                    line_index - selected_range.end + 1
                } else {
                    0
                };
                if distance > 0 {
                    let max_distance = half_viewport.max(1) as f64;
                    let dim_factor = (distance as f64 / max_distance).min(1.0) * 0.7;
                    line = Line::from(
                        line.spans
                            .into_iter()
                            .map(|span| {
                                if let Some(foreground) = span.style.fg {
                                    Span::styled(
                                        span.content,
                                        span.style
                                            .fg(theme::blend(foreground, dim_target, dim_factor)),
                                    )
                                } else {
                                    span
                                }
                            })
                            .collect::<Vec<_>>(),
                    );
                }
                if index == selected {
                    line = highlight_line_to_width(line, self.width);
                }
                visible.push(line);
            }
        }
        visible
    }

    fn height(&self, index: usize) -> usize {
        self.line_ranges[index]
            .end
            .saturating_sub(self.line_ranges[index].start)
            .max(1)
    }

    fn total_lines(&self) -> usize {
        self.line_ranges.last().map_or(0, |range| range.end)
    }

    fn max_offset(&self) -> usize {
        self.total_lines().saturating_sub(self.viewport_height)
    }
}

fn build_comment_lines(
    comments: &[Comment],
    content_width: usize,
    spinner: char,
) -> Vec<Vec<Line<'static>>> {
    let comment_max_lines = theme::COMMENT_MAX_LINES.unwrap_or(usize::MAX);
    let now = now_unix();
    comments
        .iter()
        .map(|comment| {
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
            let by = comment.by.as_deref().unwrap_or("[unknown]").to_string();
            let age = comment
                .time
                .map(|time| format_age(time, now))
                .unwrap_or_else(|| "?".to_string());
            let author_style = Style::default()
                .fg(theme::comment_indent_color(comment.depth))
                .add_modifier(Modifier::BOLD);
            let mut lines = vec![Line::from(vec![
                Span::styled(indent.clone(), indent_style),
                Span::styled(format!("{thread_marker} "), marker_style),
                Span::styled(by, author_style),
                Span::styled(format!(" · {age}"), theme::META),
            ])];

            let body_indent = format!("{indent}  ");
            let body_width = content_width.saturating_sub(indent_width + 2).max(1);
            let plain = hn_html_to_plain(&comment.text);
            if !plain.is_empty() {
                for wrapped in wrap_content(&plain, body_width, comment_max_lines) {
                    let line = match wrapped {
                        ContentLine::Normal(text) => Line::from(vec![
                            Span::styled(body_indent.clone(), indent_style),
                            Span::styled(text, Style::default().fg(theme::TEXT)),
                        ]),
                        ContentLine::Quote(text) => Line::from(vec![
                            Span::styled(body_indent.clone(), indent_style),
                            Span::styled("▎ ", theme::QUOTE_BAR),
                            Span::styled(text, theme::QUOTE),
                        ]),
                        ContentLine::Blank => {
                            Line::from(Span::styled(body_indent.clone(), indent_style))
                        }
                    };
                    lines.push(line);
                }
            }
            lines
        })
        .collect()
}

fn highlight_line_to_width(mut line: Line<'static>, width: usize) -> Line<'static> {
    let line_width = line
        .spans
        .iter()
        .map(|span| span.content.chars().count())
        .sum::<usize>();
    if line_width < width {
        line.spans.push(Span::styled(
            " ".repeat(width - line_width),
            theme::SELECTED,
        ));
    }
    line.patch_style(theme::SELECTED)
}

enum ContentLine {
    Normal(String),
    Quote(String),
    Blank,
}

fn wrap_content(content: &str, width: usize, max_lines: usize) -> Vec<ContentLine> {
    if max_lines == 0 {
        return vec![ContentLine::Normal(String::new())];
    }
    let width = width.max(1);
    let quote_width = width.saturating_sub(2).max(1);
    let mut paragraphs = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut first_line = true;

    for raw_line in content.split('\n') {
        let line = collapse_spaces(raw_line.trim());
        if line.is_empty() {
            if !current.is_empty() {
                paragraphs.push((std::mem::take(&mut current), quoted));
                first_line = true;
            }
            continue;
        }
        if first_line {
            quoted = line.starts_with('>');
            first_line = false;
        }
        let text = if quoted {
            line.trim_start_matches('>').trim_start()
        } else {
            &line
        };
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(text);
    }
    if !current.is_empty() {
        paragraphs.push((current, quoted));
    }

    let mut output = Vec::new();
    for (paragraph_index, (paragraph, quoted)) in paragraphs.iter().enumerate() {
        if paragraph_index > 0 {
            output.push(ContentLine::Blank);
            if output.len() >= max_lines {
                return output;
            }
        }
        let line_width = if *quoted { quote_width } else { width };
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            if line.is_empty() {
                line.push_str(word);
                continue;
            }
            if line.chars().count() + 1 + word.chars().count() <= line_width {
                line.push(' ');
                line.push_str(word);
                continue;
            }
            output.push(content_line(std::mem::take(&mut line), *quoted));
            if output.len() >= max_lines {
                return output;
            }
            line.push_str(word);
        }
        if !line.is_empty() {
            output.push(content_line(line, *quoted));
            if output.len() >= max_lines {
                return output;
            }
        }
    }
    if output.is_empty() {
        output.push(ContentLine::Normal(String::new()));
    }
    output
}

fn content_line(text: String, quoted: bool) -> ContentLine {
    if quoted {
        ContentLine::Quote(text)
    } else {
        ContentLine::Normal(text)
    }
}

fn collapse_spaces(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut previous_space = false;
    for character in value.chars() {
        if character.is_whitespace() {
            if !previous_space {
                output.push(' ');
            }
            previous_space = true;
        } else {
            previous_space = false;
            output.push(character);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::Comment;

    fn comment(id: u64, text: &str) -> Comment {
        Comment {
            id,
            by: Some(format!("user{id}")),
            time: Some(1),
            text: text.to_string(),
            kids: vec![],
            depth: 0,
            collapsed: false,
            children_loaded: true,
            children_loading: false,
        }
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn variable_height_comments_share_one_line_mapping() {
        let comments = vec![comment(1, "a"), comment(2, "aa bb cc"), comment(3, "d")];
        let mut layout = CommentLayout::default();

        layout.relayout(&comments, 8, 4, '⠋');

        assert_eq!(layout.line_range(0), Some(0..2));
        assert_eq!(layout.line_range(1), Some(2..5));
        assert_eq!(layout.line_range(2), Some(5..7));

        layout.ensure_visible(2);
        assert_eq!(layout.hit_test(0), Some(1));
        assert_eq!(layout.hit_test(2), Some(2));
        assert_eq!(layout.visible_lines(2).len(), 4);
    }

    #[test]
    fn over_tall_comment_is_pinned_to_its_first_line() {
        let comments = vec![comment(1, "a"), comment(2, "aa bb cc dd ee ff")];
        let mut layout = CommentLayout::default();
        layout.relayout(&comments, 8, 3, '⠋');

        layout.ensure_visible(1);

        let visible = layout.visible_lines(1);
        assert!(line_text(&visible[0]).contains("user2"));
    }

    #[test]
    fn paging_moves_by_visible_comment_heights() {
        let comments = vec![
            comment(1, "a"),
            comment(2, "aa bb cc"),
            comment(3, "d"),
            comment(4, "e"),
        ];
        let mut layout = CommentLayout::default();
        layout.relayout(&comments, 8, 4, '⠋');

        let next = layout.page_down(0);
        assert_eq!(next, 1);
        assert_eq!(layout.page_up(next), 0);
    }
}
