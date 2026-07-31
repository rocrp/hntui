use super::article_tests::{linked_story, listed_self_post, self_post};
use super::tests::{cli, comment, story, test_article_fetcher};
use super::*;
use crate::api::{InMemorySource, Sources};
use crate::config::{Config, SummarizeConfig};
use crate::input::{Action, SettingsAction, SummaryAction, TextAction};
use crate::summarizer::{
    LlmFuture, LlmSession, LlmStream, Summarizer, SummaryChunk, SummaryRequest,
};
use crate::ui::summary_overlay::SummaryState;
use futures::FutureExt;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct SuccessfulLlmStream;

impl LlmStream for SuccessfulLlmStream {
    fn start(&self, _request: SummaryRequest) -> LlmFuture {
        async move {
            Ok(LlmSession::for_test(
                "fake/model",
                vec![Ok(SummaryChunk {
                    content: "summary".to_string(),
                    reasoning: String::new(),
                })],
            ))
        }
        .boxed()
    }
}

#[derive(Clone)]
struct RecordingLlmStream {
    prompts: Arc<Mutex<Vec<String>>>,
}

impl LlmStream for RecordingLlmStream {
    fn start(&self, request: SummaryRequest) -> LlmFuture {
        self.prompts
            .lock()
            .expect("prompt recorder poisoned")
            .push(request.user_prompt().to_string());
        SuccessfulLlmStream.start(request)
    }
}

fn summarize_config(include_article: bool) -> SummarizeConfig {
    SummarizeConfig {
        model: "fake/model".to_string(),
        api_key: None,
        base_url: None,
        max_comments: 20,
        include_article,
        max_article_chars: 20_000,
        system_prompt: "Summarize".to_string(),
    }
}

fn app_with_source(
    stories: Vec<Story>,
    source: Arc<InMemorySource>,
    include_article: bool,
) -> (App, mpsc::UnboundedReceiver<AppEvent>) {
    let sources = Sources::new(source.clone(), source);
    let (tx, rx) = mpsc::unbounded_channel();
    let summarize = summarize_config(include_article);
    let directory = std::env::temp_dir().join(format!(
        "hntui-summarize-{}-{include_article}.toml",
        std::process::id()
    ));
    let config = Config::for_test_with_summarize(directory, summarize.clone());
    let summarizer = Summarizer::with_stream(Some(summarize), None, Arc::new(SuccessfulLlmStream));
    let mut app = App::new(
        cli(),
        sources,
        tx,
        None,
        config,
        summarizer,
        test_article_fetcher(),
    );
    let story_ids = stories.iter().map(|story| story.id).collect();
    app.restore_story_list_state(story_ids, stories, None);
    (app, rx)
}

fn app_with_summarize(
    stories: Vec<Story>,
    include_article: bool,
) -> (App, mpsc::UnboundedReceiver<AppEvent>) {
    let source = Arc::new(InMemorySource::new(stories.clone()).with_comments(1, vec![comment(11)]));
    app_with_source(stories, source, include_article)
}

async fn finish_summary(app: &mut App, rx: &mut mpsc::UnboundedReceiver<AppEvent>) {
    while app.tasks.is_running(TaskTarget::Summary) {
        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("summary timed out")
            .expect("app event channel closed");
        app.handle_app_event(event);
    }
}

#[tokio::test]
async fn a_self_post_summary_needs_no_fetch_and_carries_no_degrade_banner() {
    let (mut app, mut rx) = app_with_summarize(vec![self_post(1, "<p>the body")], true);
    app.apply_comments_for_story(
        self_post(1, "<p>the body"),
        StoryThread::from_comments(vec![comment(11)]),
        false,
    );

    app.handle_action(Action::Summarize);

    assert!(app.pending_summary.is_none());
    assert!(!app.tasks.is_running(TaskTarget::Article(1)));
    assert_eq!(app.summary_overlay.article_notice(), None);
    finish_summary(&mut app, &mut rx).await;
    assert_eq!(app.summary_overlay.state(), SummaryState::Done);
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
    finish_summary(&mut app, &mut rx).await;
    assert_eq!(app.summary_overlay.state(), SummaryState::Done);
}

#[tokio::test]
async fn toggling_the_article_off_summarizes_immediately_without_a_fetch() {
    let (mut app, mut rx) = app_with_summarize(vec![linked_story(1)], false);
    app.apply_comments_for_story(
        linked_story(1),
        StoryThread::from_comments(vec![comment(11)]),
        false,
    );

    app.handle_action(Action::Summarize);

    assert!(app.pending_summary.is_none());
    assert!(!app.tasks.is_running(TaskTarget::Article(1)));
    assert_eq!(app.summary_overlay.article_notice(), None);
    finish_summary(&mut app, &mut rx).await;
    assert_eq!(app.summary_overlay.state(), SummaryState::Done);
}

#[tokio::test]
async fn settings_toggle_off_preserves_the_pre_article_prompt_bytes() {
    let directory = tempfile::tempdir().expect("temp dir");
    let item = linked_story(1);
    let source =
        Arc::new(InMemorySource::new(vec![item.clone()]).with_comments(1, vec![comment(11)]));
    let sources = Sources::new(source.clone(), source);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let summarize = summarize_config(true);
    let config =
        Config::for_test_with_summarize(directory.path().join("config.toml"), summarize.clone());
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let summarizer = Summarizer::with_stream(
        Some(summarize),
        None,
        Arc::new(RecordingLlmStream {
            prompts: prompts.clone(),
        }),
    );
    let mut app = App::new(
        cli(),
        sources,
        tx,
        None,
        config,
        summarizer,
        test_article_fetcher(),
    );
    app.restore_story_list_state(vec![item.id], vec![item.clone()], None);

    app.handle_action(Action::OpenSettings);
    for _ in 0..4 {
        app.handle_action(Action::Settings(SettingsAction::MoveDown));
    }
    app.handle_action(Action::Settings(SettingsAction::Activate));
    app.handle_action(Action::Settings(SettingsAction::Edit(
        TextAction::DeleteToStart,
    )));
    for character in "false".chars() {
        app.handle_action(Action::Settings(SettingsAction::Edit(TextAction::Insert(
            character,
        ))));
    }
    app.handle_action(Action::Settings(SettingsAction::Edit(TextAction::Submit)));
    let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("settings save timed out")
        .expect("app event channel closed");
    app.handle_app_event(event);
    app.handle_action(Action::Settings(SettingsAction::CloseAndSave));

    assert!(
        !app.config
            .summarize()
            .expect("summarize config")
            .include_article
    );
    app.apply_comments_for_story(item, StoryThread::from_comments(vec![comment(11)]), false);
    app.handle_action(Action::Summarize);
    assert!(!app.tasks.is_running(TaskTarget::Article(1)));
    finish_summary(&mut app, &mut rx).await;

    assert_eq!(app.summary_overlay.state(), SummaryState::Done);
    assert_eq!(
        prompts.lock().expect("prompt recorder poisoned").as_slice(),
        ["# story 1\n\nbob: hello\n\n"]
    );
}

#[tokio::test]
async fn summarizing_from_the_story_list_runs_both_legs_in_parallel() {
    let (mut app, mut rx) = app_with_summarize(vec![linked_story(1)], true);

    app.handle_action(Action::Summarize);

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
    finish_summary(&mut app, &mut rx).await;
    assert_eq!(app.summary_overlay.state(), SummaryState::Done);
}

#[tokio::test]
async fn dismissing_summary_cancels_comment_loading_started_for_the_summary() {
    let (mut app, _rx) = app_with_summarize(vec![linked_story(1)], true);
    app.handle_action(Action::Summarize);
    assert!(app.pending_summary.is_some());
    assert!(app.tasks.is_running(TaskTarget::CommentRoots(1)));

    app.handle_action(Action::Summary(SummaryAction::Dismiss));

    assert!(app.pending_summary.is_none());
    assert!(!app.tasks.is_running(TaskTarget::Article(1)));
    assert!(!app.tasks.is_running(TaskTarget::CommentRoots(1)));
    assert!(!app.comment_loading);
    assert!(!app.summary_overlay.is_visible());
}

#[tokio::test]
async fn dismissing_summary_preserves_comment_loading_started_by_the_view() {
    let (mut app, mut rx) = app_with_summarize(vec![linked_story(1)], true);
    app.open_comments_for_selected_story();
    assert!(app.comment_list.is_empty());
    assert!(app.comment_loading);
    assert!(app.tasks.is_running(TaskTarget::CommentRoots(1)));

    app.handle_action(Action::Summarize);
    assert!(app.pending_summary.is_some());
    app.handle_action(Action::Summary(SummaryAction::Dismiss));

    assert!(app.pending_summary.is_none());
    assert!(!app.tasks.is_running(TaskTarget::Article(1)));
    assert!(app.tasks.is_running(TaskTarget::CommentRoots(1)));
    assert!(app.comment_loading);

    while app.comment_loading {
        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("comments timed out")
            .expect("app event channel closed");
        app.handle_app_event(event);
    }
    assert_eq!(app.comment_list.len(), 1);
}

#[tokio::test]
async fn a_failed_comment_load_fails_the_waiting_summary_overlay() {
    let source = Arc::new(InMemorySource::new(vec![linked_story(1)]).with_initial_error("unused"));
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
    app.restore_story_list_state(vec![1], vec![linked_story(1)], None);

    app.handle_action(Action::Summarize);
    let task = app
        .tasks
        .targets_where(|target| matches!(target, TaskTarget::CommentRoots(_)));
    assert_eq!(task, vec![TaskTarget::CommentRoots(1)]);

    for _ in 0..2 {
        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("leg timed out")
            .expect("app event channel closed");
        app.handle_app_event(event);
    }

    assert!(app.pending_summary.is_none());
}

#[tokio::test]
async fn a_zero_comment_self_post_body_completes_an_article_only_summary() {
    let item = story(1);
    let source =
        Arc::new(InMemorySource::new(vec![item.clone()]).with_thread_text(1, "<p>the ask hn body"));
    let (mut app, mut rx) = app_with_source(vec![item], source, true);

    app.handle_action(Action::Summarize);
    assert!(app.tasks.is_running(TaskTarget::Article(1)));

    for _ in 0..2 {
        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("leg timed out")
            .expect("app event channel closed");
        app.handle_app_event(event);
    }

    assert!(app.pending_summary.is_none());
    assert_eq!(app.summary_overlay.article_notice(), None);
    finish_summary(&mut app, &mut rx).await;
    assert_eq!(app.summary_overlay.state(), SummaryState::Done);
}

#[tokio::test]
async fn a_self_post_without_a_body_completes_a_comments_only_summary_without_a_warning() {
    let (mut app, mut rx) = app_with_summarize(vec![listed_self_post(1)], true);

    app.handle_action(Action::Summarize);

    for _ in 0..2 {
        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("summary leg timed out")
            .expect("app event channel closed");
        app.handle_app_event(event);
    }

    assert!(app.pending_summary.is_none());
    assert_eq!(app.summary_overlay.article_notice(), None);
    finish_summary(&mut app, &mut rx).await;
    assert_eq!(app.summary_overlay.state(), SummaryState::Done);
}
