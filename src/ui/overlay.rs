//! Geometry and chrome shared by the scrolling overlays (Summary, Article).
//!
//! Both are the same shape: a centred bordered popup, a wrapped content column,
//! a reserved scrollbar lane, and a one-line hint footer. Keeping the layout in
//! one place is why the two overlays cannot drift apart — the same reason
//! `ClampedScroll` owns their scroll geometry.

use super::clamped_scroll::ClampedScroll;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::{Block, Borders, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::Frame;
use std::time::{Duration, Instant};

/// How long a "Copied!" flash stays up after `c`.
const COPIED_FLASH: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy)]
pub(crate) struct OverlayAreas {
    pub(crate) popup: Rect,
    pub(crate) content: Rect,
    pub(crate) scrollbar: Rect,
    pub(crate) hint: Rect,
}

/// The popup box, or `None` when the terminal is too small to bother.
pub(crate) fn popup_rect(area: Rect) -> Option<Rect> {
    if area.width < 12 || area.height < 8 {
        return None;
    }
    Some(super::centered(
        area,
        (area.width * 4 / 5).max(30),
        (area.height * 4 / 5).max(10),
    ))
}

pub(crate) fn areas(area: Rect) -> Option<OverlayAreas> {
    let popup = popup_rect(area)?;
    let inner = Block::default().borders(Borders::ALL).inner(popup);
    let [body, hint] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);
    let [content, _gutter, scrollbar] = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(body);
    Some(OverlayAreas {
        popup,
        content,
        scrollbar,
        hint,
    })
}

/// Draw the scrollbar, but only when the content actually overflows — the lane
/// stays reserved either way so text never reflows when it appears.
pub(crate) fn render_scrollbar(frame: &mut Frame, area: Rect, scroll: &ClampedScroll) {
    if scroll.content_height() <= scroll.viewport_height() {
        return;
    }
    // ScrollbarState counts reachable positions. `max + 1` keeps its thumb
    // aligned with the viewport: top at offset 0, bottom at max offset.
    let mut state = ScrollbarState::new(scroll.max_offset().saturating_add(1))
        .position(scroll.offset())
        .viewport_content_length(scroll.viewport_height());
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight),
        area,
        &mut state,
    );
}

pub(crate) fn copied_recently(flash: Option<Instant>) -> bool {
    flash.is_some_and(|timestamp| timestamp.elapsed() < COPIED_FLASH)
}

/// YAML front-matter opener shared by the Summary and Article copy formats.
pub(crate) fn front_matter_title(title: &str) -> String {
    format!("title: \"{}\"\n", title.replace('"', "\\\""))
}

pub(crate) fn front_matter_date(timestamp: i64) -> String {
    let date = chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|date| date.format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    format!("date: {date}\n")
}

pub(crate) fn hn_url(story_id: u64) -> String {
    format!("https://news.ycombinator.com/item?id={story_id}")
}
