use super::tests::{completed_overlay, story};
use super::*;
use crate::summarizer::SummaryEvent;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

fn streaming_overlay(summary: &str) -> SummaryOverlay {
    let mut overlay = SummaryOverlay::default();
    overlay.begin(&story(), 2);
    overlay.handle_event(SummaryEvent::Chunk {
        content: summary.to_string(),
        reasoning: String::new(),
    });
    overlay
}

fn render_overlay(overlay: &mut SummaryOverlay, width: u16, height: u16) -> (Buffer, Rect) {
    let area = Rect::new(0, 0, width, height);
    let content_area = content_area(area).expect("test terminal should fit summary popup");
    overlay.set_viewport(content_area.width, content_area.height);

    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("create test terminal");
    terminal
        .draw(|frame| render(frame, overlay, '⠋'))
        .expect("render summary overlay");

    (terminal.backend().buffer().clone(), content_area)
}

fn many_paragraphs(count: usize) -> String {
    (1..=count)
        .map(|number| format!("paragraph {number}"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[test]
fn overflowing_summary_renders_a_scrollbar_on_the_content_right_edge() {
    let mut overlay = completed_overlay(&many_paragraphs(12));

    let (buffer, content_area) = render_overlay(&mut overlay, 50, 15);

    let right_edge = content_area.right() - 1;
    assert_eq!(buffer[(right_edge, content_area.top())].symbol(), "▲");
    assert_eq!(
        buffer[(right_edge, content_area.bottom() - 1)].symbol(),
        "▼"
    );
}

#[test]
fn summary_that_fits_the_viewport_does_not_render_a_scrollbar() {
    let mut overlay = completed_overlay("short");

    let (buffer, content_area) = render_overlay(&mut overlay, 50, 15);

    let right_edge = content_area.right() - 1;
    for row in content_area.top()..content_area.bottom() {
        assert_eq!(buffer[(right_edge, row)].symbol(), " ");
    }
}

#[test]
fn scrollbar_thumb_moves_from_the_top_to_the_bottom_with_the_summary() {
    let mut overlay = completed_overlay(&many_paragraphs(12));

    let (top_buffer, content_area) = render_overlay(&mut overlay, 50, 15);
    let right_edge = content_area.right() - 1;
    assert_eq!(
        top_buffer[(right_edge, content_area.top() + 1)].symbol(),
        "█"
    );
    assert_eq!(
        top_buffer[(right_edge, content_area.bottom() - 2)].symbol(),
        "║"
    );

    overlay.scroll_down(usize::MAX);
    let (bottom_buffer, _) = render_overlay(&mut overlay, 50, 15);
    assert_eq!(
        bottom_buffer[(right_edge, content_area.top() + 1)].symbol(),
        "║"
    );
    assert_eq!(
        bottom_buffer[(right_edge, content_area.bottom() - 2)].symbol(),
        "█"
    );
}

#[test]
fn streaming_growth_shrinks_the_scrollbar_thumb() {
    let mut overlay = streaming_overlay(&many_paragraphs(8));

    let (before_buffer, content_area) = render_overlay(&mut overlay, 50, 15);
    let right_edge = content_area.right() - 1;
    let before_thumb_height = (content_area.top()..content_area.bottom())
        .filter(|&row| before_buffer[(right_edge, row)].symbol() == "█")
        .count();

    overlay.handle_event(SummaryEvent::Chunk {
        content: format!("\n\n{}", many_paragraphs(12)),
        reasoning: String::new(),
    });
    let (after_buffer, _) = render_overlay(&mut overlay, 50, 15);
    let after_thumb_height = (content_area.top()..content_area.bottom())
        .filter(|&row| after_buffer[(right_edge, row)].symbol() == "█")
        .count();

    assert!(
        after_thumb_height < before_thumb_height,
        "scrollbar thumb should shrink as streaming content grows"
    );
}
