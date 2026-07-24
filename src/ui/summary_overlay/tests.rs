use super::*;
use crate::api::types::Story;
use crate::summarizer::SummaryEvent;

pub(super) fn story() -> Story {
    Story {
        id: 42,
        title: "A story".to_string(),
        url: Some("https://example.com".to_string()),
        score: 99,
        by: "alice".to_string(),
        time: 1_700_000_000,
        comment_count: 2,
        kids: vec![1, 2],
    }
}

#[test]
fn reducer_accumulates_reasoning_then_content_without_mixing_them() {
    let mut overlay = SummaryOverlay::default();
    overlay.begin(&story(), 2);

    overlay.handle_event(SummaryEvent::Started {
        model: "fake/model".to_string(),
    });
    overlay.handle_event(SummaryEvent::Chunk {
        content: String::new(),
        reasoning: "thinking".to_string(),
    });
    overlay.handle_event(SummaryEvent::Chunk {
        content: "hello ".to_string(),
        reasoning: String::new(),
    });
    overlay.handle_event(SummaryEvent::Chunk {
        content: "world".to_string(),
        reasoning: "ignored after content".to_string(),
    });
    overlay.handle_event(SummaryEvent::Complete);

    assert_eq!(overlay.state(), SummaryState::Done);
    assert_eq!(overlay.reasoning, "thinking");
    assert_eq!(overlay.summary, "hello world");
    assert_eq!(overlay.model_name, "fake/model");
}

#[test]
fn clipboard_text_contains_story_metadata_and_raw_markdown() {
    let mut overlay = SummaryOverlay::default();
    overlay.begin(&story(), 2);
    overlay.handle_event(SummaryEvent::Started {
        model: "fake/model".to_string(),
    });
    overlay.handle_event(SummaryEvent::Chunk {
        content: "# Summary".to_string(),
        reasoning: String::new(),
    });

    let text = overlay.copy_text();

    assert_eq!(
        text,
        "---\n\
         title: \"A story\"\n\
         source: https://example.com\n\
         hn: https://news.ycombinator.com/item?id=42\n\
         score: 99\n\
         author: alice\n\
         comments: 2\n\
         model: fake/model\n\
         date: 2023-11-14\n\
         ---\n\n\
         # Summary"
    );
}

pub(super) fn completed_overlay(summary: &str) -> SummaryOverlay {
    let mut overlay = SummaryOverlay::default();
    overlay.begin(&story(), 2);
    overlay.handle_event(SummaryEvent::Chunk {
        content: summary.to_string(),
        reasoning: String::new(),
    });
    overlay.handle_event(SummaryEvent::Complete);
    overlay
}

#[test]
fn scrolling_stops_when_the_last_wrapped_line_reaches_the_viewport_bottom() {
    let mut overlay = completed_overlay("one\n\ntwo\n\nthree\n\nfour");
    overlay.set_viewport(40, 3);

    overlay.scroll_down(usize::MAX);

    assert_eq!(overlay.wrapped_line_count(), 7);
    assert_eq!(overlay.scroll_offset(), 4);
}

#[test]
fn viewport_resize_reflows_content_and_reclamps_the_offset() {
    let mut overlay = completed_overlay("11111 22222 33333");
    overlay.set_viewport(5, 1);
    overlay.scroll_down(usize::MAX);
    assert_eq!(overlay.scroll_offset(), 2);

    overlay.set_viewport(80, 1);

    assert_eq!(overlay.wrapped_line_count(), 1);
    assert_eq!(overlay.scroll_offset(), 0);

    let mut overlay = completed_overlay("one\n\ntwo\n\nthree\n\nfour");
    overlay.set_viewport(40, 2);
    overlay.scroll_down(usize::MAX);
    assert_eq!(overlay.scroll_offset(), 5);

    overlay.set_viewport(40, 6);

    assert_eq!(overlay.scroll_offset(), 1);
}

#[test]
fn content_shorter_than_the_viewport_does_not_scroll() {
    let mut overlay = completed_overlay("short");
    overlay.set_viewport(40, 5);

    overlay.scroll_down(usize::MAX);

    assert_eq!(overlay.scroll_offset(), 0);
}

#[test]
fn reasoning_stream_stays_pinned_to_its_latest_line() {
    let mut overlay = SummaryOverlay::default();
    overlay.begin(&story(), 2);
    overlay.set_viewport(40, 3);

    overlay.handle_event(SummaryEvent::Chunk {
        content: String::new(),
        reasoning: "one\ntwo\nthree\nfour".to_string(),
    });

    assert_eq!(overlay.wrapped_line_count(), 6);
    assert_eq!(overlay.scroll_offset(), 3);

    overlay.scroll_up(2);

    assert_eq!(overlay.scroll_offset(), 3);
    overlay.go_top();
    assert_eq!(overlay.scroll_offset(), 3);
}

#[test]
fn first_summary_content_resets_to_top_then_streaming_growth_holds_position() {
    let mut overlay = SummaryOverlay::default();
    overlay.begin(&story(), 2);
    overlay.set_viewport(40, 3);
    overlay.handle_event(SummaryEvent::Chunk {
        content: String::new(),
        reasoning: "one\ntwo\nthree\nfour".to_string(),
    });
    assert_eq!(overlay.scroll_offset(), 3);

    overlay.handle_event(SummaryEvent::Chunk {
        content: "one\n\ntwo\n\nthree\n\nfour".to_string(),
        reasoning: String::new(),
    });

    assert_eq!(overlay.scroll_offset(), 0);
    overlay.scroll_down(2);
    assert_eq!(overlay.scroll_offset(), 2);
    let initial_extent = overlay.wrapped_line_count();

    overlay.handle_event(SummaryEvent::Chunk {
        content: "\n\nfive\n\nsix".to_string(),
        reasoning: String::new(),
    });

    assert!(overlay.wrapped_line_count() > initial_extent);
    assert_eq!(overlay.scroll_offset(), 2);
    overlay.scroll_down(usize::MAX);
    assert!(overlay.scroll_offset() > 2);
}
