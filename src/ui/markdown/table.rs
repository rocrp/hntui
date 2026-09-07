//! Laying GFM tables out as aligned columns.
//!
//! A table cannot be streamed the way the rest of the document is: column
//! widths are only known once every cell has been seen. Cells are collected
//! here as styled spans, then emitted as padded lines when the table closes.

use crate::ui::theme;
use pulldown_cmark::Alignment;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Blank columns separating one table column from the next.
const COLUMN_GAP: usize = 2;

/// The narrowest a column is squeezed to. Two cells of content plus the
/// ellipsis that says the rest was cut.
const MIN_COLUMN_WIDTH: usize = 3;

/// One cell's inline content, already styled by the document renderer.
type Cell = Vec<Span<'static>>;

pub(super) struct TableBuilder {
    alignments: Vec<Alignment>,
    header: Vec<Cell>,
    body: Vec<Vec<Cell>>,
    row: Vec<Cell>,
    in_header: bool,
}

impl TableBuilder {
    pub(super) fn new(alignments: Vec<Alignment>) -> Self {
        Self {
            alignments,
            header: Vec::new(),
            body: Vec::new(),
            row: Vec::new(),
            in_header: false,
        }
    }

    pub(super) fn begin_row(&mut self, header: bool) {
        self.in_header = header;
        self.row = Vec::new();
    }

    pub(super) fn push_cell(&mut self, cell: Cell) {
        self.row.push(cell);
    }

    pub(super) fn end_row(&mut self) {
        let row = std::mem::take(&mut self.row);
        if self.in_header {
            self.header = row;
        } else if !row.is_empty() {
            self.body.push(row);
        }
    }

    /// Lays the collected cells out to fit `viewport` columns. A `viewport` of
    /// zero means the width is not known yet, so columns keep their natural
    /// widths and the caller's wrapping decides.
    pub(super) fn render(self, viewport: u16) -> Vec<Line<'static>> {
        let columns = self
            .body
            .iter()
            .map(Vec::len)
            .chain(std::iter::once(self.header.len()))
            .max()
            .unwrap_or(0);
        if columns == 0 {
            return Vec::new();
        }

        let mut widths = vec![0usize; columns];
        for row in std::iter::once(&self.header).chain(&self.body) {
            for (column, cell) in row.iter().enumerate() {
                widths[column] = widths[column].max(cell_width(cell));
            }
        }
        fit_columns(&mut widths, viewport);

        let alignment = |column: usize| {
            self.alignments
                .get(column)
                .copied()
                .unwrap_or(Alignment::None)
        };

        let mut lines = Vec::new();
        if !self.header.is_empty() {
            lines.push(row_line(&self.header, &widths, alignment, true));
            lines.push(rule_line(&widths));
        }
        for row in &self.body {
            lines.push(row_line(row, &widths, alignment, false));
        }
        lines
    }
}

/// Squeezes the widest column one cell at a time until the row fits. Spending
/// the cuts on whichever column is currently widest keeps narrow columns
/// readable instead of shaving every column equally.
fn fit_columns(widths: &mut [usize], viewport: u16) {
    let viewport = usize::from(viewport);
    if viewport == 0 {
        return;
    }
    let gaps = COLUMN_GAP * widths.len().saturating_sub(1);
    let Some(budget) = viewport.checked_sub(gaps) else {
        // Narrower than the gaps alone; nothing sensible left to fit.
        return;
    };
    while widths.iter().sum::<usize>() > budget {
        let widest = widths
            .iter()
            .copied()
            .enumerate()
            .max_by_key(|(index, width)| (*width, std::cmp::Reverse(*index)))
            .map(|(index, _)| index)
            .expect("a table has at least one column");
        if widths[widest] <= MIN_COLUMN_WIDTH {
            // Every column is at its floor; the table overflows rather than
            // losing a column entirely.
            break;
        }
        widths[widest] -= 1;
    }
}

fn row_line(
    row: &[Cell],
    widths: &[usize],
    alignment: impl Fn(usize) -> Alignment,
    header: bool,
) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (column, width) in widths.iter().copied().enumerate() {
        if column > 0 {
            spans.push(Span::raw(" ".repeat(COLUMN_GAP)));
        }
        let empty = Vec::new();
        let cell = row.get(column).unwrap_or(&empty);
        let mut cell = truncated_cell(cell, width);
        if header {
            for span in &mut cell {
                span.style = span.style.add_modifier(Modifier::BOLD);
            }
        }
        let slack = width.saturating_sub(cell_width(&cell));
        let (before, after) = match alignment(column) {
            Alignment::Right => (slack, 0),
            Alignment::Center => (slack / 2, slack - slack / 2),
            Alignment::Left | Alignment::None => (0, slack),
        };
        if before > 0 {
            spans.push(Span::raw(" ".repeat(before)));
        }
        spans.extend(cell);
        if after > 0 {
            spans.push(Span::raw(" ".repeat(after)));
        }
    }
    Line::from(spans)
}

fn rule_line(widths: &[usize]) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (column, width) in widths.iter().copied().enumerate() {
        if column > 0 {
            spans.push(Span::raw(" ".repeat(COLUMN_GAP)));
        }
        spans.push(Span::styled("─".repeat(width), theme::META));
    }
    Line::from(spans)
}

/// The display width of a cell, which is what alignment has to reason about —
/// a CJK glyph occupies two terminal cells, not one.
fn cell_width(cell: &[Span<'static>]) -> usize {
    cell.iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum()
}

/// Cuts a cell down to `limit` display columns, marking the cut with an
/// ellipsis so a truncated value never reads as a complete one.
fn truncated_cell(cell: &[Span<'static>], limit: usize) -> Cell {
    if cell_width(cell) <= limit {
        return cell.to_vec();
    }
    if limit == 0 {
        return Vec::new();
    }
    let budget = limit - 1;
    let mut kept: Cell = Vec::new();
    let mut used = 0usize;
    'cells: for span in cell {
        let mut text = String::new();
        for character in span.content.chars() {
            let width = UnicodeWidthChar::width(character).unwrap_or(0);
            if used + width > budget {
                if !text.is_empty() {
                    kept.push(Span::styled(text, span.style));
                }
                break 'cells;
            }
            used += width;
            text.push(character);
        }
        if !text.is_empty() {
            kept.push(Span::styled(text, span.style));
        }
    }
    kept.push(Span::styled("…".to_string(), theme::META));
    kept
}
