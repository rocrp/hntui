use super::*;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

fn story() -> Story {
    Story {
        id: 42,
        title: "A story".to_string(),
        url: Some("https://example.com/post".to_string()),
        text: None,
        score: 99,
        by: "alice".to_string(),
        time: 1_700_000_000,
        comment_count: 2,
        kids: vec![1, 2],
    }
}

fn article(content: &str) -> Article {
    Article {
        title: Some("Extracted title".to_string()),
        content: content.to_string(),
    }
}

fn done_overlay(content: &str) -> ArticleOverlay {
    let mut overlay = ArticleOverlay::default();
    overlay.show(&story(), article(content));
    overlay
}

fn many_paragraphs(count: usize) -> String {
    (1..=count)
        .map(|number| format!("paragraph {number}"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn render_overlay(
    overlay: &mut ArticleOverlay,
    width: u16,
    height: u16,
) -> (Buffer, overlay::OverlayAreas) {
    let area = Rect::new(0, 0, width, height);
    let areas = overlay::areas(area).expect("test terminal should fit the article popup");
    overlay.set_viewport(areas.content.width, areas.content.height);

    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("create test terminal");
    terminal
        .draw(|frame| render(frame, overlay, '⠋'))
        .expect("render article overlay");

    (terminal.backend().buffer().clone(), areas)
}

#[test]
fn scrolling_stops_when_the_last_wrapped_line_reaches_the_viewport_bottom() {
    let mut overlay = done_overlay("one\n\ntwo\n\nthree\n\nfour");
    overlay.set_viewport(40, 3);

    overlay.scroll_down(usize::MAX);

    assert_eq!(overlay.wrapped_line_count(), 7);
    assert_eq!(overlay.scroll_offset(), 4);
}

#[test]
fn viewport_resize_reflows_content_and_reclamps_the_offset() {
    let mut overlay = done_overlay("11111 22222 33333");
    overlay.set_viewport(5, 1);
    overlay.scroll_down(usize::MAX);
    assert_eq!(overlay.scroll_offset(), 2);

    overlay.set_viewport(80, 1);

    assert_eq!(overlay.wrapped_line_count(), 1);
    assert_eq!(overlay.scroll_offset(), 0);
}

#[test]
fn content_shorter_than_the_viewport_does_not_scroll() {
    let mut overlay = done_overlay("short");
    overlay.set_viewport(40, 5);

    overlay.scroll_down(usize::MAX);

    assert_eq!(overlay.scroll_offset(), 0);
}

#[test]
fn go_top_and_go_bottom_reach_both_ends() {
    let mut overlay = done_overlay("one\n\ntwo\n\nthree\n\nfour");
    overlay.set_viewport(40, 3);

    overlay.go_bottom();
    assert_eq!(overlay.scroll_offset(), 4);

    overlay.go_top();
    assert_eq!(overlay.scroll_offset(), 0);
}

#[test]
fn a_settled_fetch_replaces_the_loading_state_and_resets_the_scroll() {
    let mut overlay = ArticleOverlay::default();
    overlay.begin(&story());
    overlay.set_viewport(40, 3);
    assert_eq!(overlay.state(), ArticleState::Loading);
    overlay.scroll_down(usize::MAX);

    overlay.finish(article(&many_paragraphs(10)));

    assert_eq!(overlay.state(), ArticleState::Done);
    assert_eq!(overlay.scroll_offset(), 0);
    assert!(overlay.wrapped_line_count() > 3);
}

#[test]
fn dismissing_returns_the_overlay_to_idle() {
    let mut overlay = done_overlay("body");

    overlay.dismiss();

    assert_eq!(overlay.state(), ArticleState::Idle);
    assert!(!overlay.is_visible());
}

#[test]
fn an_error_overlay_is_scrollable_and_keeps_the_story_link() {
    let mut overlay = ArticleOverlay::default();
    overlay.begin(&story());
    overlay.fail("localwebrs not found — install it".to_string());

    assert_eq!(overlay.state(), ArticleState::Error);
    assert_eq!(overlay.story_url(), Some("https://example.com/post"));
    assert_eq!(overlay.story_id(), 42);
}

#[test]
fn clipboard_text_pairs_front_matter_with_the_article_markdown() {
    let overlay = done_overlay("# Heading\n\nbody");

    assert_eq!(
        overlay.copy_text(),
        "---\n\
         title: \"Extracted title\"\n\
         source: https://example.com/post\n\
         hn: https://news.ycombinator.com/item?id=42\n\
         date: 2023-11-14\n\
         ---\n\n\
         # Heading\n\nbody"
    );
}

#[test]
fn front_matter_falls_back_to_the_story_title_when_extraction_found_none() {
    let mut overlay = ArticleOverlay::default();
    overlay.show(
        &story(),
        Article {
            title: None,
            content: "body".to_string(),
        },
    );

    assert!(overlay.copy_text().contains("title: \"A story\""));
}

#[test]
fn overflowing_article_renders_in_the_reserved_scrollbar_lane() {
    let mut overlay = done_overlay(&many_paragraphs(12));

    let (buffer, areas) = render_overlay(&mut overlay, 50, 15);

    let right_edge = areas.scrollbar.left();
    assert_eq!(buffer[(right_edge, areas.scrollbar.top())].symbol(), "▲");
    assert_eq!(
        buffer[(right_edge, areas.scrollbar.bottom() - 1)].symbol(),
        "▼"
    );
}

#[test]
fn article_that_fits_the_viewport_does_not_render_a_scrollbar() {
    let mut overlay = done_overlay("short");

    let (buffer, areas) = render_overlay(&mut overlay, 50, 15);

    let right_edge = areas.scrollbar.left();
    for row in areas.scrollbar.top()..areas.scrollbar.bottom() {
        assert_eq!(buffer[(right_edge, row)].symbol(), " ");
    }
}

#[test]
fn the_loading_line_shows_elapsed_seconds_and_how_to_cancel() {
    let mut overlay = ArticleOverlay::default();
    overlay.begin(&story());

    let (buffer, areas) = render_overlay(&mut overlay, 60, 15);
    let title: String = (areas.popup.left()..areas.popup.right())
        .map(|column| buffer[(column, areas.popup.top())].symbol().to_string())
        .collect();
    let first_line: String = (areas.content.left()..areas.content.right())
        .map(|column| buffer[(column, areas.content.top())].symbol().to_string())
        .collect();

    assert!(title.contains("A story"), "unexpected title: {title:?}");
    assert!(title.contains("example.com"), "unexpected title: {title:?}");
    assert!(
        first_line.contains("fetching article… 0s"),
        "unexpected loading line: {first_line:?}"
    );
    assert!(
        first_line.contains("Esc to cancel"),
        "unexpected loading line: {first_line:?}"
    );
}

#[test]
fn the_title_bar_carries_the_story_title_and_its_domain() {
    let mut overlay = done_overlay("body");

    let (buffer, areas) = render_overlay(&mut overlay, 60, 15);
    let title: String = (areas.popup.left()..areas.popup.right())
        .map(|column| buffer[(column, areas.popup.top())].symbol().to_string())
        .collect();

    assert!(title.contains("A story"), "unexpected title: {title:?}");
    assert!(title.contains("example.com"), "unexpected title: {title:?}");
}

#[test]
fn a_self_post_overlay_labels_its_source_as_self() {
    let mut self_post = story();
    self_post.url = None;
    let mut overlay = ArticleOverlay::default();
    overlay.show(&self_post, article("body"));

    let (buffer, areas) = render_overlay(&mut overlay, 60, 15);
    let title: String = (areas.popup.left()..areas.popup.right())
        .map(|column| buffer[(column, areas.popup.top())].symbol().to_string())
        .collect();

    assert!(title.contains("(self)"), "unexpected title: {title:?}");
}
