use super::tests::{cli, comment, key, story, test_article_fetcher};
use super::*;
use crate::api::{InMemorySource, Sources};
use crate::config::Config;
use crate::input::{Action, InputLayer, SummaryAction};
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

fn linked_story(id: u64) -> Story {
    let mut story = story(id);
    story.url = Some(format!("https://example.com/{id}"));
    story
}

fn self_post(id: u64, body: &str) -> Story {
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

#[test]
fn v_on_a_story_with_neither_link_nor_body_reports_no_article() {
    let (mut app, _rx) = app_with_stories(vec![story(1)]);

    app.handle_key(key(KeyCode::Char('v')));

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

/// An app whose summarize is configured (so the article toggle is live) and
/// whose LLM stream is a fake — no network, no real subprocess.
fn app_with_summarize(
    stories: Vec<Story>,
    include_article: bool,
) -> (App, mpsc::UnboundedReceiver<AppEvent>) {
    let source = Arc::new(InMemorySource::new(stories.clone()).with_comments(1, vec![comment(11)]));
    let sources = Sources::new(source.clone(), source);
    let (tx, rx) = mpsc::unbounded_channel();
    let directory = std::env::temp_dir().join(format!(
        "hntui-summarize-{}-{include_article}.toml",
        std::process::id()
    ));
    let config = Config::for_test_with_summarize(
        directory,
        crate::config::SummarizeConfig {
            model: "fake/model".to_string(),
            api_key: None,
            base_url: None,
            max_comments: 20,
            include_article,
            max_article_chars: 20_000,
            system_prompt: "Summarize".to_string(),
        },
    );
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

#[tokio::test]
async fn a_self_post_summary_needs_no_fetch_and_carries_no_degrade_banner() {
    let (mut app, _rx) = app_with_summarize(vec![self_post(1, "<p>the body")], true);
    app.apply_comments_for_story(
        self_post(1, "<p>the body"),
        StoryThread::from_comments(vec![comment(11)]),
        false,
    );

    app.handle_action(Action::Summarize);

    assert!(app.pending_summary.is_none());
    assert!(!app.tasks.is_running(TaskTarget::Article(1)));
    assert_eq!(app.summary_overlay.article_notice(), None);
    assert!(app.tasks.is_running(TaskTarget::Summary));
}

#[tokio::test]
async fn a_failed_article_leg_degrades_to_comments_only_with_a_banner() {
    let (mut app, mut rx) = app_with_summarize(vec![linked_story(1)], true);
    app.apply_comments_for_story(
        linked_story(1),
        StoryThread::from_comments(vec![comment(11)]),
        false,
    );

    app.handle_action(Action::Summarize);
    assert!(app.pending_summary.is_some());
    assert!(app.tasks.is_running(TaskTarget::Article(1)));

    let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("article fetch timed out")
        .expect("app event channel closed");
    app.handle_app_event(event);

    assert!(app.pending_summary.is_none());
    let notice = app
        .summary_overlay
        .article_notice()
        .expect("degrade banner set");
    assert!(
        notice.contains("cargo install"),
        "unexpected notice: {notice}"
    );
    assert!(app.tasks.is_running(TaskTarget::Summary));
}

#[tokio::test]
async fn toggling_the_article_off_summarizes_immediately_without_a_fetch() {
    let (mut app, _rx) = app_with_summarize(vec![linked_story(1)], false);
    app.apply_comments_for_story(
        linked_story(1),
        StoryThread::from_comments(vec![comment(11)]),
        false,
    );

    app.handle_action(Action::Summarize);

    assert!(app.pending_summary.is_none());
    assert!(!app.tasks.is_running(TaskTarget::Article(1)));
    assert_eq!(app.summary_overlay.article_notice(), None);
}

#[tokio::test]
async fn summarizing_from_the_story_list_runs_both_legs_in_parallel() {
    let (mut app, mut rx) = app_with_summarize(vec![linked_story(1)], true);

    app.handle_action(Action::Summarize);

    // Both legs are outstanding at once, and the overlay says so.
    assert!(app.tasks.is_running(TaskTarget::Article(1)));
    assert!(app.tasks.is_running(TaskTarget::CommentRoots(1)));
    assert_eq!(app.summary_overlay.state(), SummaryState::Loading);
    assert!(app.summary_overlay.is_visible());

    for _ in 0..2 {
        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("leg timed out")
            .expect("app event channel closed");
        app.handle_app_event(event);
    }

    assert!(app.pending_summary.is_none());
    assert!(app.tasks.is_running(TaskTarget::Summary));
}

#[tokio::test]
async fn dismissing_while_the_legs_settle_drops_the_pending_summary() {
    let (mut app, _rx) = app_with_summarize(vec![linked_story(1)], true);
    app.handle_action(Action::Summarize);
    assert!(app.pending_summary.is_some());

    app.handle_action(Action::Summary(SummaryAction::Dismiss));

    assert!(app.pending_summary.is_none());
    assert!(!app.tasks.is_running(TaskTarget::Article(1)));
    assert!(!app.summary_overlay.is_visible());
}

#[tokio::test]
async fn a_failed_comment_load_fails_the_waiting_summary_overlay() {
    let source = Arc::new(InMemorySource::new(vec![linked_story(1)]).with_initial_error("unused"));
    let sources = Sources::new(source.clone(), source);
    let (tx, mut rx) = mpsc::unbounded_channel();
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
    app.restore_story_list_state(vec![1], vec![linked_story(1)], None);

    app.handle_action(Action::Summarize);
    let task = app
        .tasks
        .targets_where(|target| matches!(target, TaskTarget::CommentRoots(_)));
    assert_eq!(task, vec![TaskTarget::CommentRoots(1)]);

    // Drain both legs; the comment load resolves empty, the article fails.
    for _ in 0..2 {
        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("leg timed out")
            .expect("app event channel closed");
        app.handle_app_event(event);
    }

    assert!(app.pending_summary.is_none());
}

/// A story whose body only exists on its discussion — the hackerweb shape, and
/// the default backend. `story.text` is None until the thread arrives.
fn listed_self_post(id: u64) -> Story {
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

#[tokio::test]
async fn summarizing_a_listed_self_post_feeds_its_body_to_the_article_leg() {
    let source = Arc::new(
        InMemorySource::new(vec![listed_self_post(1)])
            .with_comments(1, vec![comment(11)])
            .with_thread_text(1, "<p>the ask hn body"),
    );
    let sources = Sources::new(source.clone(), source);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let config = Config::for_test_with_summarize(
        std::env::temp_dir().join("hntui-selfpost-summary.toml"),
        crate::config::SummarizeConfig {
            model: "fake/model".to_string(),
            api_key: None,
            base_url: None,
            max_comments: 20,
            include_article: true,
            max_article_chars: 20_000,
            system_prompt: "Summarize".to_string(),
        },
    );
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

    app.handle_action(Action::Summarize);
    assert!(app.tasks.is_running(TaskTarget::Article(1)));

    for _ in 0..2 {
        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("leg timed out")
            .expect("app event channel closed");
        app.handle_app_event(event);
    }

    // The body was found, so the summary is not degraded.
    assert!(app.pending_summary.is_none());
    assert_eq!(app.summary_overlay.article_notice(), None);
}

#[test]
fn a_job_posting_has_nothing_to_ask_for() {
    // No link, no body, no discussion to hide a body in.
    let (mut app, _rx) = app_with_stories(vec![story(1)]);

    app.handle_key(key(KeyCode::Char('v')));

    assert_eq!(app.article_overlay.state(), ArticleState::Error);
    assert!(!app.tasks.is_running(TaskTarget::Article(1)));
}
