use super::tests::{cli, comment, key, story, test_article_fetcher};
use super::*;
use crate::api::{InMemorySource, Sources};
use crate::config::Config;
use crate::input::InputLayer;
use crate::summarizer::Summarizer;
use crate::ui::article_overlay::ArticleState;
use crossterm::event::KeyCode;
use std::sync::Arc;

fn app_with_stories(stories: Vec<Story>) -> (App, mpsc::UnboundedReceiver<AppEvent>) {
    let source = Arc::new(InMemorySource::new(stories.clone()));
    let sources = Sources::new(source.clone(), source);
    let (tx, rx) = mpsc::unbounded_channel();
    let config = Config::for_test(std::env::temp_dir().join("hntui-test-config.toml"));
    let summarizer = Summarizer::new(None, None, reqwest::Client::new());
    let article_fetcher = test_article_fetcher();
    let mut app = App::new(
        cli(),
        sources,
        tx,
        None,
        config,
        summarizer,
        article_fetcher,
    );
    let story_ids = stories.iter().map(|story| story.id).collect();
    app.restore_story_list_state(story_ids, stories, None);
    (app, rx)
}

pub(super) fn linked_story(id: u64) -> Story {
    let mut story = story(id);
    story.url = Some(format!("https://example.com/{id}"));
    story
}

pub(super) fn self_post(id: u64, body: &str) -> Story {
    let mut story = story(id);
    story.text = Some(body.to_string());
    story
}

#[test]
fn v_on_a_self_post_renders_its_body_without_spawning_a_fetch() {
    let (mut app, _rx) = app_with_stories(vec![self_post(1, "<p>Hello&nbsp;world")]);

    app.handle_key(key(KeyCode::Char('v')));

    assert_eq!(app.input_layer(), InputLayer::Article);
    assert_eq!(app.article_overlay.state(), ArticleState::Done);
    assert!(!app.tasks.is_running(TaskTarget::Article(1)));
}

#[tokio::test]
async fn v_on_a_story_with_neither_link_nor_body_reports_no_article() {
    let (mut app, mut rx) = app_with_stories(vec![story(1)]);

    app.handle_key(key(KeyCode::Char('v')));

    let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("thread lookup timed out")
        .expect("app event channel closed");
    app.handle_app_event(event);

    assert_eq!(app.article_overlay.state(), ArticleState::Error);
    assert!(!app.tasks.is_running(TaskTarget::Article(1)));
}

#[tokio::test]
async fn v_on_a_linked_story_fetches_and_a_failure_lands_in_the_overlay() {
    let (mut app, mut rx) = app_with_stories(vec![linked_story(1)]);

    app.handle_key(key(KeyCode::Char('v')));
    assert_eq!(app.article_overlay.state(), ArticleState::Loading);
    assert!(app.tasks.is_running(TaskTarget::Article(1)));

    // The test fetcher points at a binary that does not exist.
    let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("article fetch timed out")
        .expect("app event channel closed");
    app.handle_app_event(event);

    assert_eq!(app.article_overlay.state(), ArticleState::Error);
    assert!(!app.tasks.is_running(TaskTarget::Article(1)));
}

#[tokio::test]
async fn esc_in_the_article_overlay_cancels_the_fetch_and_closes() {
    let (mut app, _rx) = app_with_stories(vec![linked_story(1)]);
    app.handle_key(key(KeyCode::Char('v')));
    assert!(app.tasks.is_running(TaskTarget::Article(1)));

    app.handle_key(key(KeyCode::Esc));

    assert!(!app.article_overlay.is_visible());
    assert!(!app.tasks.is_running(TaskTarget::Article(1)));
    assert_eq!(app.input_layer(), InputLayer::View);
}

#[tokio::test]
async fn a_second_v_on_the_same_story_serves_the_stored_article() {
    let (mut app, mut rx) = app_with_stories(vec![linked_story(1)]);
    app.handle_key(key(KeyCode::Char('v')));
    // Settle the fetch by hand — the real subprocess is out of scope here.
    let AppEvent::TaskFailed { task, .. } = rx.recv().await.expect("fetch event") else {
        panic!("expected the missing-binary fetch to fail");
    };
    app.handle_app_event(AppEvent::ArticleLoaded {
        task,
        story_id: 1,
        article: crate::article::Article {
            title: Some("T".to_string()),
            content: "stored body".to_string(),
        },
    });
    app.handle_key(key(KeyCode::Esc));

    app.handle_key(key(KeyCode::Char('v')));

    assert_eq!(app.article_overlay.state(), ArticleState::Done);
    assert!(!app.tasks.is_running(TaskTarget::Article(1)));
}

#[test]
fn v_from_the_comments_view_targets_the_open_story() {
    let (mut app, _rx) = app_with_stories(vec![story(1), self_post(2, "<p>second body")]);
    app.apply_comments_for_story(
        self_post(2, "<p>second body"),
        StoryThread::from_comments(vec![comment(21)]),
        true,
    );
    assert_eq!(app.view, View::Comments);

    app.handle_key(key(KeyCode::Char('v')));

    assert_eq!(app.article_overlay.state(), ArticleState::Done);
    assert_eq!(app.article_overlay.story_id(), 2);
}

#[test]
fn the_article_overlay_takes_the_input_layer_below_help() {
    let (mut app, _rx) = app_with_stories(vec![self_post(1, "<p>body")]);
    app.handle_key(key(KeyCode::Char('v')));
    assert_eq!(app.input_layer(), InputLayer::Article);

    app.handle_key(key(KeyCode::Char('?')));

    assert_eq!(app.input_layer(), InputLayer::Help);
    assert_eq!(app.help_focus(), HelpFocus::Article);
}

/// A story whose body only exists on its discussion — the hackerweb shape, and
/// the default backend. `story.text` is None until the thread arrives.
pub(super) fn listed_self_post(id: u64) -> Story {
    let mut story = story(id);
    story.comment_count = 1;
    story.kids = vec![id * 10 + 1];
    story
}

#[tokio::test]
async fn v_on_a_listed_self_post_resolves_its_body_through_the_source() {
    let source = Arc::new(
        InMemorySource::new(vec![listed_self_post(1)])
            .with_comments(1, vec![comment(11)])
            .with_thread_text(1, "<p>the ask hn body"),
    );
    let sources = Sources::new(source.clone(), source);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let config = Config::for_test(std::env::temp_dir().join("hntui-test-config.toml"));
    let summarizer = Summarizer::new(None, None, reqwest::Client::new());
    let mut app = App::new(
        cli(),
        sources,
        tx,
        None,
        config,
        summarizer,
        test_article_fetcher(),
    );
    app.restore_story_list_state(vec![1], vec![listed_self_post(1)], None);

    app.handle_key(key(KeyCode::Char('v')));

    // The listing carried no body, so the overlay waits on the Source — not on
    // localwebrs, and not on an "no article" error.
    assert_eq!(app.article_overlay.state(), ArticleState::Loading);
    assert!(app.tasks.is_running(TaskTarget::Article(1)));

    let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("self-post body timed out")
        .expect("app event channel closed");
    app.handle_app_event(event);

    assert_eq!(app.article_overlay.state(), ArticleState::Done);
}

#[tokio::test]
async fn v_on_a_zero_comment_listed_self_post_still_checks_for_its_body() {
    let source =
        Arc::new(InMemorySource::new(vec![story(1)]).with_thread_text(1, "<p>fresh ask hn body"));
    let sources = Sources::new(source.clone(), source);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let config = Config::for_test(std::env::temp_dir().join("hntui-test-config.toml"));
    let summarizer = Summarizer::new(None, None, reqwest::Client::new());
    let mut app = App::new(
        cli(),
        sources,
        tx,
        None,
        config,
        summarizer,
        test_article_fetcher(),
    );
    app.restore_story_list_state(vec![1], vec![story(1)], None);

    app.handle_key(key(KeyCode::Char('v')));

    assert_eq!(app.article_overlay.state(), ArticleState::Loading);
    assert!(app.tasks.is_running(TaskTarget::Article(1)));
    let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("self-post body timed out")
        .expect("app event channel closed");
    app.handle_app_event(event);

    assert_eq!(app.article_overlay.state(), ArticleState::Done);
}

#[tokio::test]
async fn a_listed_self_post_with_no_body_reports_no_article() {
    let source = Arc::new(
        InMemorySource::new(vec![listed_self_post(1)]).with_comments(1, vec![comment(11)]),
    );
    let sources = Sources::new(source.clone(), source);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let config = Config::for_test(std::env::temp_dir().join("hntui-test-config.toml"));
    let summarizer = Summarizer::new(None, None, reqwest::Client::new());
    let mut app = App::new(
        cli(),
        sources,
        tx,
        None,
        config,
        summarizer,
        test_article_fetcher(),
    );
    app.restore_story_list_state(vec![1], vec![listed_self_post(1)], None);

    app.handle_key(key(KeyCode::Char('v')));
    let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("thread fetch timed out")
        .expect("app event channel closed");
    app.handle_app_event(event);

    assert_eq!(app.article_overlay.state(), ArticleState::Error);
}

#[test]
fn a_prefetched_discussion_serves_the_self_post_body_without_a_task() {
    let (mut app, _rx) = app_with_stories(vec![listed_self_post(1)]);
    let selected = app.story_list_state.selected().unwrap_or(0);
    app.prefetched_comments_cache.insert(
        1,
        StoryThread {
            text: Some("<p>prefetched body".to_string()),
            comments: vec![comment(11)],
        },
        &app.stories,
        selected,
    );

    app.handle_key(key(KeyCode::Char('v')));

    assert_eq!(app.article_overlay.state(), ArticleState::Done);
    assert!(!app.tasks.is_running(TaskTarget::Article(1)));
    // Reading the body must not consume the prefetched comments.
    assert!(app.prefetched_comments_cache.contains(1));
}
