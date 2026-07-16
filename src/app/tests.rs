use super::test_support::{controlled_root_request, ControlledStorySource};
use super::*;
use crate::api::{ApiBackend, InMemorySource, Sources};
use crate::input::{Action, SummaryAction};
use crate::summarizer::{Summarizer, SummaryEvent};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use futures::StreamExt;
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::Arc;

fn story(id: u64) -> Story {
    Story {
        id,
        title: format!("story {id}"),
        url: None,
        score: 10,
        by: "alice".to_string(),
        time: 1,
        comment_count: 0,
        kids: vec![],
    }
}

fn comment(id: u64) -> CommentNode {
    CommentNode {
        comment: crate::api::types::Comment {
            id,
            by: Some("bob".to_string()),
            time: Some(1),
            text: "hello".to_string(),
            kids: vec![],
            depth: 0,
            collapsed: false,
            children_loaded: true,
            children_loading: false,
        },
        children: vec![],
    }
}

fn cli() -> Cli {
    Cli {
        count: NonZeroUsize::new(30).unwrap(),
        page_size: NonZeroUsize::new(30).unwrap(),
        cache_size: NonZeroUsize::new(100).unwrap(),
        concurrency: NonZeroUsize::new(4).unwrap(),
        no_file_cache: true,
        file_cache_dir: None,
        log_file: None,
        file_cache_ttl_secs: NonZeroU64::new(3600).unwrap(),
        api_backend: ApiBackend::HackerWeb,
        base_url: None,
        config: None,
        env_file: None,
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn left_click(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

#[tokio::test]
async fn refresh_loads_initial_stories_through_the_app_event_seam() {
    let source = Arc::new(InMemorySource::new(vec![story(1), story(2)]));
    let sources = Sources::new(source.clone(), source);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let config = Config::for_test(std::env::temp_dir().join("hntui-test-config.toml"));
    let summarizer = Summarizer::new(None, None, reqwest::Client::new());
    let mut app = App::new(cli(), sources, tx, None, config, summarizer);

    app.handle_action(Action::Refresh);
    let event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("story load timed out")
        .expect("app event channel closed");
    app.handle_app_event(event);

    assert_eq!(
        app.stories.iter().map(|story| story.id).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert!(!app.story_loading);
    assert_eq!(app.last_error, None);
}

#[tokio::test]
async fn opening_a_story_loads_comments_from_the_in_memory_source() {
    let source = Arc::new(InMemorySource::new(vec![story(1)]).with_comments(1, vec![comment(11)]));
    let sources = Sources::new(source.clone(), source);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let config = Config::for_test(std::env::temp_dir().join("hntui-test-config.toml"));
    let summarizer = Summarizer::new(None, None, reqwest::Client::new());
    let mut app = App::new(cli(), sources, tx, None, config, summarizer);
    app.handle_action(Action::Refresh);
    app.handle_app_event(rx.recv().await.expect("stories event"));

    app.handle_action(Action::Enter);
    app.handle_app_event(rx.recv().await.expect("comments event"));

    assert_eq!(app.current_story.as_ref().map(|story| story.id), Some(1));
    assert_eq!(
        app.comment_list
            .iter()
            .map(|comment| comment.id)
            .collect::<Vec<_>>(),
        vec![11]
    );
    assert!(!app.comment_loading);
}

#[tokio::test]
async fn stale_story_result_is_dropped_after_a_refresh() {
    let source = Arc::new(InMemorySource::new(vec![story(1)]));
    let sources = Sources::new(source.clone(), source);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let config = Config::for_test(std::env::temp_dir().join("hntui-test-config.toml"));
    let summarizer = Summarizer::new(None, None, reqwest::Client::new());
    let mut app = App::new(cli(), sources, tx, None, config, summarizer);

    app.handle_action(Action::Refresh);
    let stale = rx.recv().await.expect("stale stories event");
    app.handle_action(Action::Refresh);
    let current = rx.recv().await.expect("current stories event");

    app.handle_app_event(stale);
    assert!(app.stories.is_empty());
    assert!(app.story_loading);

    app.handle_app_event(current);
    assert_eq!(app.stories[0].id, 1);
    assert!(!app.story_loading);
}

#[tokio::test]
async fn source_error_is_surfaced_by_the_app() {
    let source =
        Arc::new(InMemorySource::new(Vec::new()).with_initial_error("fixture source failed"));
    let sources = Sources::new(source.clone(), source);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let config = Config::for_test(std::env::temp_dir().join("hntui-test-config.toml"));
    let summarizer = Summarizer::new(None, None, reqwest::Client::new());
    let mut app = App::new(cli(), sources, tx, None, config, summarizer);

    app.handle_action(Action::Refresh);
    app.handle_app_event(rx.recv().await.expect("error event"));

    assert_eq!(app.last_error.as_deref(), Some("fixture source failed"));
    assert!(!app.story_loading);
}

#[tokio::test]
async fn search_uses_the_in_memory_search_source_without_pagination_state() {
    let source = Arc::new(InMemorySource::new(vec![story(1)]).with_search("rust", vec![story(9)]));
    let sources = Sources::new(source.clone(), source);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let config = Config::for_test(std::env::temp_dir().join("hntui-test-config.toml"));
    let summarizer = Summarizer::new(None, None, reqwest::Client::new());
    let mut app = App::new(cli(), sources, tx, None, config, summarizer);
    app.search_query = "rust".to_string();

    app.submit_search();
    app.handle_app_event(rx.recv().await.expect("search event"));

    assert_eq!(
        app.stories.iter().map(|story| story.id).collect::<Vec<_>>(),
        vec![9]
    );
    assert!(!app.has_more_stories);
}

#[tokio::test]
async fn expanding_a_comment_loads_children_from_the_in_memory_source() {
    let mut parent = comment(11);
    parent.comment.kids = vec![12];
    parent.comment.collapsed = true;
    parent.comment.children_loaded = false;
    let mut child = comment(12);
    child.comment.depth = 1;
    let source = Arc::new(
        InMemorySource::new(vec![story(1)])
            .with_comments(1, vec![parent])
            .with_children(vec![child]),
    );
    let sources = Sources::new(source.clone(), source);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let config = Config::for_test(std::env::temp_dir().join("hntui-test-config.toml"));
    let summarizer = Summarizer::new(None, None, reqwest::Client::new());
    let mut app = App::new(cli(), sources, tx, None, config, summarizer);
    app.handle_action(Action::Refresh);
    app.handle_app_event(rx.recv().await.expect("stories event"));
    app.handle_action(Action::Enter);
    app.handle_app_event(rx.recv().await.expect("comments event"));

    app.handle_action(Action::Expand);
    app.handle_app_event(rx.recv().await.expect("children event"));

    assert_eq!(
        app.comment_list
            .iter()
            .map(|comment| comment.id)
            .collect::<Vec<_>>(),
        vec![11, 12]
    );
}

#[tokio::test]
async fn settings_popup_open_edit_save_flows_only_through_actions() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("config.toml");
    let source = Arc::new(InMemorySource::default());
    let sources = Sources::new(source.clone(), source);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let config = Config::for_test(path.clone());
    let summarizer = Summarizer::new(None, None, reqwest::Client::new());
    let mut app = App::new(cli(), sources, tx, None, config, summarizer);

    app.handle_action(Action::OpenSettings);
    app.handle_key(key(KeyCode::Enter));
    for character in "openai/test".chars() {
        app.handle_key(key(KeyCode::Char(character)));
    }
    app.handle_key(key(KeyCode::Enter));
    let event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("settings save timed out")
        .expect("settings event channel closed");
    app.handle_app_event(event);

    assert_eq!(
        app.config
            .summarize()
            .expect("saved summarize config")
            .model,
        "openai/test"
    );
    assert!(path.exists());
    assert!(!app.settings_popup.as_ref().expect("popup open").dirty);
}

#[tokio::test]
async fn feed_popup_open_move_select_uses_the_same_action_ladder() {
    let source = Arc::new(InMemorySource::default());
    let sources = Sources::new(source.clone(), source);
    let (tx, _rx) = mpsc::unbounded_channel();
    let config = Config::for_test(std::env::temp_dir().join("hntui-test-config.toml"));
    let summarizer = Summarizer::new(None, None, reqwest::Client::new());
    let mut app = App::new(cli(), sources, tx, None, config, summarizer);

    app.handle_action(Action::OpenFeedFilter);
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Enter));

    assert_eq!(app.current_feed, FeedKind::New);
    assert!(app.feed_filter_popup.is_none());
}

#[test]
fn mouse_selection_changes_flow_through_indexed_actions() {
    let source = Arc::new(InMemorySource::default());
    let sources = Sources::new(source.clone(), source);
    let (tx, _rx) = mpsc::unbounded_channel();
    let config = Config::for_test(std::env::temp_dir().join("hntui-test-config.toml"));
    let summarizer = Summarizer::new(None, None, reqwest::Client::new());
    let mut app = App::new(cli(), sources, tx, None, config, summarizer);
    app.restore_story_list_state(vec![1, 2], vec![story(1), story(2)], None);
    app.layout_areas.list_area = Rect::new(0, 0, 80, 10);

    app.handle_mouse(left_click(1, 1));

    assert_eq!(app.story_list_state.selected(), Some(1));

    app.apply_comments_for_story(story(2), vec![comment(11), comment(12)], true);
    app.comment_layout.relayout(&app.comment_list, 80, 10, '⠋');
    app.handle_mouse(left_click(1, 2));

    assert_eq!(app.comment_list_state.selected(), Some(1));
}

#[test]
fn summary_copy_failure_is_surfaced_on_the_app() {
    let source = Arc::new(InMemorySource::default());
    let sources = Sources::new(source.clone(), source);
    let (tx, _rx) = mpsc::unbounded_channel();
    let config = Config::for_test(std::env::temp_dir().join("hntui-test-config.toml"));
    let summarizer = Summarizer::new(None, None, reqwest::Client::new());
    let mut app = App::new(cli(), sources, tx, None, config, summarizer);

    app.handle_action(Action::Summary(SummaryAction::Copy));

    assert_eq!(
        app.last_error.as_deref(),
        Some("clipboard: summary is empty")
    );
}

#[tokio::test]
async fn refresh_cancels_comment_prefetch_at_the_action_boundary() {
    let mut item = story(1);
    item.comment_count = 1;
    item.kids = vec![11];
    let (request, mut control) = controlled_root_request();
    let story_source = Arc::new(ControlledStorySource::new(
        vec![item.clone()],
        vec![request],
    ));
    let search_source = Arc::new(InMemorySource::default());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let config = Config::for_test(std::env::temp_dir().join("hntui-test-config.toml"));
    let summarizer = Summarizer::new(None, None, reqwest::Client::new());
    let mut app = App::new(
        cli(),
        Sources::new(story_source, search_source),
        tx,
        None,
        config,
        summarizer,
    );
    app.restore_story_list_state(vec![item.id], vec![item], None);
    app.last_user_activity = Instant::now() - IDLE_PREFETCH_DELAY;

    app.maybe_prefetch_comments();
    assert_eq!(control.started.await.expect("prefetch started"), 1);

    app.handle_action(Action::Refresh);
    tokio::time::timeout(Duration::from_secs(1), &mut control.dropped)
        .await
        .expect("prefetch was not cancelled")
        .expect("prefetch drop signal closed");

    assert!(!app.tasks.is_running(TaskTarget::CommentRoots(1)));
    assert!(!app.comment_loading);
    assert_eq!(app.pending_summarize_story_id, None);
    assert!(control.result.send(Ok(vec![comment(11)])).is_err());

    app.handle_app_event(rx.recv().await.expect("replacement stories event"));
    assert_eq!(app.stories[0].id, 1);
}

#[tokio::test]
async fn foreground_navigation_supersedes_same_story_prefetch() {
    let mut item = story(1);
    item.comment_count = 1;
    item.kids = vec![11];
    let (prefetch_request, mut prefetch) = controlled_root_request();
    let (foreground_request, foreground) = controlled_root_request();
    let story_source = Arc::new(ControlledStorySource::new(
        vec![item.clone()],
        vec![prefetch_request, foreground_request],
    ));
    let search_source = Arc::new(InMemorySource::default());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let config = Config::for_test(std::env::temp_dir().join("hntui-test-config.toml"));
    let summarizer = Summarizer::new(None, None, reqwest::Client::new());
    let mut app = App::new(
        cli(),
        Sources::new(story_source, search_source),
        tx,
        None,
        config,
        summarizer,
    );
    app.restore_story_list_state(vec![item.id], vec![item], None);
    app.last_user_activity = Instant::now() - IDLE_PREFETCH_DELAY;

    app.maybe_prefetch_comments();
    assert_eq!(prefetch.started.await.expect("prefetch started"), 1);

    app.handle_action(Action::Enter);
    tokio::time::timeout(Duration::from_secs(1), &mut prefetch.dropped)
        .await
        .expect("prefetch was not superseded")
        .expect("prefetch drop signal closed");
    assert_eq!(
        foreground.started.await.expect("foreground load started"),
        1
    );
    assert!(prefetch.result.send(Ok(vec![comment(10)])).is_err());

    foreground
        .result
        .send(Ok(vec![comment(11)]))
        .expect("send foreground comments");
    app.handle_app_event(rx.recv().await.expect("foreground comments event"));

    assert_eq!(app.view, View::Comments);
    assert_eq!(
        app.comment_list
            .iter()
            .map(|comment| comment.id)
            .collect::<Vec<_>>(),
        vec![11]
    );
    assert!(!app.tasks.is_running(TaskTarget::CommentRoots(1)));
}

#[tokio::test]
async fn dismissing_summary_cancels_stream_and_rejects_queued_chunks() {
    let source = Arc::new(InMemorySource::default());
    let sources = Sources::new(source.clone(), source);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let config = Config::for_test(std::env::temp_dir().join("hntui-test-config.toml"));
    let summarizer = Summarizer::new(None, None, reqwest::Client::new());
    let mut app = App::new(cli(), sources, tx, None, config, summarizer);
    let item = story(1);
    app.summary_overlay.begin(&item, 1);

    let (stream_tx, mut stream_rx) =
        tokio::sync::mpsc::unbounded_channel::<anyhow::Result<SummaryEvent>>();
    let stream = async_stream::stream! {
        while let Some(event) = stream_rx.recv().await {
            yield event;
        }
    }
    .boxed();
    app.tasks
        .spawn_stream(TaskTarget::Summary, stream, |task, event| {
            AppEvent::Summary { task, event }
        });

    stream_tx
        .send(Ok(SummaryEvent::Started {
            model: "fake/model".to_string(),
        }))
        .expect("send started event");
    app.handle_app_event(rx.recv().await.expect("started event"));
    stream_tx
        .send(Ok(SummaryEvent::Chunk {
            content: "stale content".to_string(),
            reasoning: String::new(),
        }))
        .expect("send queued chunk");
    let stale = rx.recv().await.expect("queued chunk event");

    app.handle_action(Action::Summary(SummaryAction::Dismiss));
    tokio::time::timeout(Duration::from_secs(1), stream_tx.closed())
        .await
        .expect("summary stream was not cancelled");
    app.handle_app_event(stale);

    assert_eq!(app.summary_overlay.state(), SummaryState::Idle);
    assert!(!app.summary_overlay.is_visible());
    assert!(!app.tasks.is_running(TaskTarget::Summary));
}
