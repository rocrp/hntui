use super::*;
use crate::api::types::Story;
use crate::summarizer::{SummaryEvent, SummaryStats};
use std::time::Duration;

/// The overlay body as plain text, the way a reader sees it.
fn rendered_text(overlay: &SummaryOverlay) -> String {
    overlay
        .content_lines('|')
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn story() -> Story {
    Story {
        id: 42,
        title: "A story".to_string(),
        url: Some("https://example.com".to_string()),
        text: None,
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
        resolved_model: None,
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
    overlay.handle_event(SummaryEvent::Complete { stats: None });

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
        resolved_model: None,
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
    overlay.handle_event(SummaryEvent::Complete { stats: None });
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

// --- #30: ResolvedModel -----------------------------------------------------

fn started(model: &str, resolved: Option<&str>) -> SummaryEvent {
    SummaryEvent::Started {
        model: model.to_string(),
        resolved_model: resolved.map(str::to_string),
    }
}

#[test]
fn the_title_names_both_models_when_a_proxy_resolved_the_request() {
    let mut overlay = SummaryOverlay::default();
    overlay.begin(&story(), 3);

    overlay.handle_event(started("smolserver/summary", Some("gpt-5!high")));

    assert_eq!(overlay.model_label(), "smolserver/summary → gpt-5!high");
}

#[test]
fn the_title_names_one_model_when_nothing_resolved_it() {
    let mut overlay = SummaryOverlay::default();
    overlay.begin(&story(), 3);

    overlay.handle_event(started("gemini/flash", None));
    assert_eq!(overlay.model_label(), "gemini/flash");

    // A server that echoes the request back is not telling us anything new.
    overlay.handle_event(started("gemini/flash", Some("gemini/flash")));
    assert_eq!(overlay.model_label(), "gemini/flash");
}

#[test]
fn a_new_summary_forgets_the_previous_resolved_model() {
    let mut overlay = SummaryOverlay::default();
    overlay.begin(&story(), 3);
    overlay.handle_event(started("smolserver/summary", Some("gpt-5!high")));

    overlay.begin(&story(), 3);

    assert_eq!(overlay.model_label(), "");
}

#[test]
fn copied_front_matter_carries_the_resolved_model_only_when_it_differs() {
    let mut overlay = SummaryOverlay::default();
    overlay.begin(&story(), 3);
    overlay.handle_event(started("smolserver/summary", Some("gpt-5!high")));
    overlay.handle_event(SummaryEvent::Chunk {
        content: "summary".to_string(),
        reasoning: String::new(),
    });
    overlay.handle_event(SummaryEvent::Complete { stats: None });

    let copied = overlay.copy_text();
    assert!(copied.contains("model: smolserver/summary\n"), "{copied}");
    assert!(copied.contains("resolved_model: gpt-5!high\n"), "{copied}");

    overlay.begin(&story(), 3);
    overlay.handle_event(started("gemini/flash", None));
    overlay.handle_event(SummaryEvent::Complete { stats: None });
    let copied = overlay.copy_text();
    assert!(copied.contains("model: gemini/flash\n"), "{copied}");
    assert!(!copied.contains("resolved_model:"), "{copied}");
}

// --- #31: stats, truncation, partial text on error --------------------------

fn stats(estimated: bool, truncated: bool) -> SummaryStats {
    SummaryStats {
        duration: Duration::from_millis(12_300),
        ttft: Some(Duration::from_millis(800)),
        input_tokens: 9_182,
        output_tokens: 1_104,
        estimated,
        truncated,
    }
}

fn finished(overlay: &mut SummaryOverlay, stats: Option<SummaryStats>) {
    overlay.handle_event(SummaryEvent::Chunk {
        content: "the summary".to_string(),
        reasoning: String::new(),
    });
    overlay.handle_event(SummaryEvent::Complete { stats });
}

#[test]
fn the_stats_line_reports_what_went_in_and_what_it_cost() {
    let mut overlay = SummaryOverlay::default();
    overlay.begin(&story(), 220);
    overlay.set_comment_count(220);
    overlay.set_article_included(true);
    finished(&mut overlay, Some(stats(false, false)));

    assert_eq!(
        overlay.stats_line().as_deref(),
        Some("220 comments · article ✓ · 12.3s · ttft 800ms · 9.2k→1.1k tok")
    );
}

#[test]
fn estimated_token_counts_are_left_out_rather_than_shown_as_fact() {
    let mut overlay = SummaryOverlay::default();
    overlay.begin(&story(), 220);
    overlay.set_comment_count(220);
    overlay.set_article_included(false);
    finished(&mut overlay, Some(stats(true, false)));

    let line = overlay.stats_line().expect("a finished summary has stats");
    assert!(
        !line.contains("tok"),
        "a chars/4 guess is not a measurement: {line}"
    );
    assert!(line.contains("article ✗"), "{line}");
}

#[test]
fn there_are_no_stats_before_a_summary_finishes() {
    let mut overlay = SummaryOverlay::default();
    overlay.begin(&story(), 3);
    overlay.handle_event(SummaryEvent::Chunk {
        content: "partial".to_string(),
        reasoning: String::new(),
    });

    assert_eq!(overlay.stats_line(), None);
    assert!(!overlay.truncated());
}

#[test]
fn a_truncated_answer_is_flagged_in_the_body_and_the_copy() {
    let mut overlay = SummaryOverlay::default();
    overlay.begin(&story(), 3);
    finished(&mut overlay, Some(stats(false, true)));

    assert!(overlay.truncated());
    let rendered = rendered_text(&overlay);
    assert!(rendered.contains("⚠ output truncated"), "{rendered}");

    let copied = overlay.copy_text();
    assert!(copied.contains("truncated: true\n"), "{copied}");
    assert!(copied.contains("duration: 12.3s\n"), "{copied}");
}

#[test]
fn a_complete_answer_carries_no_truncation_warning() {
    let mut overlay = SummaryOverlay::default();
    overlay.begin(&story(), 3);
    finished(&mut overlay, Some(stats(false, false)));

    let rendered = rendered_text(&overlay);
    assert!(!rendered.contains("truncated"), "{rendered}");
    assert!(!overlay.copy_text().contains("truncated"));
}

#[test]
fn a_failure_partway_through_keeps_what_already_streamed() {
    let mut overlay = SummaryOverlay::default();
    overlay.begin(&story(), 3);
    overlay.handle_event(SummaryEvent::Chunk {
        content: "half an answer".to_string(),
        reasoning: String::new(),
    });

    overlay.fail("connection reset".to_string());

    let rendered = rendered_text(&overlay);
    assert!(rendered.contains("half an answer"), "{rendered}");
    assert!(rendered.contains("connection reset"), "{rendered}");
}

#[test]
fn a_failure_before_any_content_shows_only_the_error() {
    let mut overlay = SummaryOverlay::default();
    overlay.begin(&story(), 3);

    overlay.fail("cannot reach host".to_string());

    let rendered = rendered_text(&overlay);
    assert!(rendered.contains("cannot reach host"), "{rendered}");
    assert_eq!(rendered.trim(), "cannot reach host");
}
