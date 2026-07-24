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

fn render_overlay(overlay: &mut SummaryOverlay, width: u16, height: u16) -> (Buffer, SummaryAreas) {
    let area = Rect::new(0, 0, width, height);
    let areas = summary_areas(area).expect("test terminal should fit summary popup");
    overlay.set_viewport(areas.content.width, areas.content.height);

    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("create test terminal");
    terminal
        .draw(|frame| render(frame, overlay, '⠋'))
        .expect("render summary overlay");

    (terminal.backend().buffer().clone(), areas)
}

fn many_paragraphs(count: usize) -> String {
    (1..=count)
        .map(|number| format!("paragraph {number}"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[test]
fn overflowing_summary_renders_in_the_reserved_scrollbar_lane() {
    let mut overlay = completed_overlay(&many_paragraphs(12));

    let (buffer, areas) = render_overlay(&mut overlay, 50, 15);

    let right_edge = areas.scrollbar.left();
    assert_eq!(buffer[(right_edge, areas.scrollbar.top())].symbol(), "▲");
    assert_eq!(
        buffer[(right_edge, areas.scrollbar.bottom() - 1)].symbol(),
        "▼"
    );
}

#[test]
fn overflowing_summary_keeps_a_blank_gutter_before_the_scrollbar() {
    let summary = format!("{}\n\n{}", "x".repeat(37), many_paragraphs(12));
    let mut overlay = completed_overlay(&summary);

    let (buffer, areas) = render_overlay(&mut overlay, 50, 15);

    let gutter = areas.content.right();
    assert_eq!(buffer[(gutter, areas.content.top())].symbol(), " ");
}

#[test]
fn summary_that_fits_the_viewport_does_not_render_a_scrollbar() {
    let mut overlay = completed_overlay("short");

    let (buffer, areas) = render_overlay(&mut overlay, 50, 15);

    let right_edge = areas.scrollbar.left();
    for row in areas.scrollbar.top()..areas.scrollbar.bottom() {
        assert_eq!(buffer[(right_edge, row)].symbol(), " ");
    }
}

#[test]
fn summary_reserves_the_gutter_before_content_overflows() {
    let mut overlay = completed_overlay(&"x".repeat(37));

    let (buffer, areas) = render_overlay(&mut overlay, 50, 15);

    let gutter = areas.content.right();
    let scrollbar = areas.scrollbar.left();
    assert_eq!(overlay.wrapped_line_count(), 2);
    assert_eq!(buffer[(gutter, areas.content.top())].symbol(), " ");
    assert_eq!(
        buffer[(areas.content.left(), areas.content.top() + 1)].symbol(),
        "x"
    );
    assert_eq!(buffer[(scrollbar, areas.scrollbar.top())].symbol(), " ");
}

#[test]
fn streaming_scrollbar_appearance_does_not_reflow_existing_text() {
    let mut overlay = streaming_overlay(&"x".repeat(37));

    let (before_buffer, areas) = render_overlay(&mut overlay, 50, 15);
    let text_and_gutter_end = areas.scrollbar.left();
    let before = (areas.content.top()..areas.content.top() + 2)
        .flat_map(|row| {
            let buffer = &before_buffer;
            (areas.content.left()..text_and_gutter_end)
                .map(move |column| buffer[(column, row)].symbol().to_string())
        })
        .collect::<Vec<_>>();

    overlay.handle_event(SummaryEvent::Chunk {
        content: format!("\n\n{}", many_paragraphs(12)),
        reasoning: String::new(),
    });
    let (after_buffer, _) = render_overlay(&mut overlay, 50, 15);
    let after = (areas.content.top()..areas.content.top() + 2)
        .flat_map(|row| {
            let buffer = &after_buffer;
            (areas.content.left()..text_and_gutter_end)
                .map(move |column| buffer[(column, row)].symbol().to_string())
        })
        .collect::<Vec<_>>();

    assert_eq!(before, after);
    assert_eq!(
        before_buffer[(areas.scrollbar.left(), areas.scrollbar.top())].symbol(),
        " "
    );
    assert_eq!(
        after_buffer[(areas.scrollbar.left(), areas.scrollbar.top())].symbol(),
        "▲"
    );
}

#[test]
fn scrollbar_thumb_moves_from_the_top_to_the_bottom_with_the_summary() {
    let mut overlay = completed_overlay(&many_paragraphs(12));

    let (top_buffer, areas) = render_overlay(&mut overlay, 50, 15);
    let right_edge = areas.scrollbar.left();
    assert_eq!(
        top_buffer[(right_edge, areas.scrollbar.top() + 1)].symbol(),
        "█"
    );
    assert_eq!(
        top_buffer[(right_edge, areas.scrollbar.bottom() - 2)].symbol(),
        "║"
    );

    overlay.scroll_down(usize::MAX);
    let (bottom_buffer, _) = render_overlay(&mut overlay, 50, 15);
    assert_eq!(
        bottom_buffer[(right_edge, areas.scrollbar.top() + 1)].symbol(),
        "║"
    );
    assert_eq!(
        bottom_buffer[(right_edge, areas.scrollbar.bottom() - 2)].symbol(),
        "█"
    );
}

#[test]
fn streaming_growth_shrinks_the_scrollbar_thumb() {
    let mut overlay = streaming_overlay(&many_paragraphs(8));

    let (before_buffer, areas) = render_overlay(&mut overlay, 50, 15);
    let right_edge = areas.scrollbar.left();
    let before_thumb_height = (areas.scrollbar.top()..areas.scrollbar.bottom())
        .filter(|&row| before_buffer[(right_edge, row)].symbol() == "█")
        .count();

    overlay.handle_event(SummaryEvent::Chunk {
        content: format!("\n\n{}", many_paragraphs(12)),
        reasoning: String::new(),
    });
    let (after_buffer, _) = render_overlay(&mut overlay, 50, 15);
    let after_thumb_height = (areas.scrollbar.top()..areas.scrollbar.bottom())
        .filter(|&row| after_buffer[(right_edge, row)].symbol() == "█")
        .count();

    assert!(
        after_thumb_height < before_thumb_height,
        "scrollbar thumb should shrink as streaming content grows"
    );
}
