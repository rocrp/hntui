//! Every copy goes through the injected clipboard seam — these watch it.

use super::tests::{cli, comment, story, test_article_fetcher};
use super::*;
use crate::api::{InMemorySource, Sources};
use crate::clipboard::RecordingClipboard;
use crate::config::Config;
use crate::input::{Action, ArticleAction, SummaryAction};
use crate::summarizer::{Summarizer, SummaryEvent};
use std::sync::Arc;

fn app_with(clipboard: Arc<RecordingClipboard>) -> App {
    let source = Arc::new(InMemorySource::default());
    let sources = Sources::new(source.clone(), source);
    let (tx, _rx) = mpsc::unbounded_channel();
    let config = Config::for_test(std::env::temp_dir().join("hntui-test-config.toml"));
    let summarizer = Summarizer::new(None, None, reqwest::Client::new());
    App::new(
        cli(),
        sources,
        tx,
        None,
        config,
        summarizer,
        test_article_fetcher(),
    )
    .with_clipboard(clipboard)
}

fn app_with_summary(clipboard: Arc<RecordingClipboard>) -> App {
    let mut app = app_with(clipboard);
    app.summary_overlay.begin(&story(42), 2);
    app.summary_overlay.handle_event(SummaryEvent::Chunk {
        content: "the summary".to_string(),
        reasoning: String::new(),
    });
    app.summary_overlay
        .handle_event(SummaryEvent::Complete { stats: None });
    app
}

fn app_with_article(clipboard: Arc<RecordingClipboard>) -> App {
    let mut app = app_with(clipboard);
    app.article_overlay.show(
        &story(42),
        crate::article::Article {
            title: None,
            content: "the article".to_string(),
            effective_url: None,
        },
    );
    app
}

#[test]
fn copying_a_comment_goes_through_the_seam() {
    let clipboard = Arc::new(RecordingClipboard::default());
    let mut app = app_with(clipboard.clone());
    app.apply_comments_for_story(
        story(1),
        crate::api::StoryThread::from_comments(vec![comment(11)]),
        true,
    );
    app.comment_list_state.select(Some(0));

    app.handle_action(Action::CopyComment);

    assert_eq!(clipboard.last_copied().as_deref(), Some("bob: hello"));
    assert!(app.copied_flash.is_some());
    assert_eq!(app.last_error, None);
}

#[test]
fn copying_a_summary_goes_through_the_seam() {
    let clipboard = Arc::new(RecordingClipboard::default());
    let mut app = app_with_summary(clipboard.clone());

    app.handle_action(Action::Summary(SummaryAction::Copy));

    let copied = clipboard.last_copied().expect("summary copied");
    assert!(copied.starts_with("---\ntitle: \"story 42\""), "{copied}");
    assert!(copied.ends_with("the summary"), "{copied}");
    assert_eq!(app.last_error, None);
}

#[test]
fn copying_an_article_goes_through_the_seam() {
    let clipboard = Arc::new(RecordingClipboard::default());
    let mut app = app_with_article(clipboard.clone());

    app.handle_action(Action::Article(ArticleAction::Copy));

    let copied = clipboard.last_copied().expect("article copied");
    assert!(copied.ends_with("the article"), "{copied}");
    assert_eq!(app.last_error, None);
}

#[test]
fn a_failing_clipboard_is_reported_and_does_not_flash_copied() {
    let clipboard = Arc::new(RecordingClipboard::failing("no clipboard here"));
    let mut app = app_with_summary(clipboard.clone());

    app.handle_action(Action::Summary(SummaryAction::Copy));

    assert_eq!(
        app.last_error.as_deref(),
        Some("clipboard: no clipboard here")
    );
    assert!(clipboard.copied().is_empty());
}

#[test]
fn an_empty_summary_never_reaches_the_clipboard() {
    let clipboard = Arc::new(RecordingClipboard::default());
    let mut app = app_with(clipboard.clone());

    app.handle_action(Action::Summary(SummaryAction::Copy));

    assert_eq!(
        app.last_error.as_deref(),
        Some("clipboard: summary is empty")
    );
    assert!(clipboard.copied().is_empty());
}
