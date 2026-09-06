//! `H`: what reaches the Paste, what reaches the clipboard, and what the
//! footer says about it.

use super::tests::{cli, comment, story, test_article_fetcher};
use super::*;
use crate::api::{InMemorySource, Sources};
use crate::clipboard::RecordingClipboard;
use crate::config::{Config, SummarizeConfig};
use crate::handoff::{HandoffStatus, RecordingPasteService};
use crate::input::Action;
use crate::summarizer::{Summarizer, SummaryEvent};
use std::sync::Arc;

pub(super) struct Harness {
    pub app: App,
    pub clipboard: Arc<RecordingClipboard>,
    rx: mpsc::UnboundedReceiver<AppEvent>,
}

impl Harness {
    pub(super) fn new(pastes: Arc<RecordingPasteService>) -> Self {
        Self::build(&pastes, Arc::new(RecordingClipboard::default()), None)
    }

    pub(super) fn with_clipboard(
        pastes: Arc<RecordingPasteService>,
        clipboard: Arc<RecordingClipboard>,
    ) -> Self {
        Self::build(&pastes, clipboard, None)
    }

    pub(super) fn with_summarize(
        pastes: Arc<RecordingPasteService>,
        summarize: SummarizeConfig,
    ) -> Self {
        Self::build(
            &pastes,
            Arc::new(RecordingClipboard::default()),
            Some(summarize),
        )
    }

    fn build(
        pastes: &Arc<RecordingPasteService>,
        clipboard: Arc<RecordingClipboard>,
        summarize: Option<SummarizeConfig>,
    ) -> Self {
        let source = Arc::new(InMemorySource::default());
        let sources = Sources::new(source.clone(), source);
        let (tx, rx) = mpsc::unbounded_channel();
        let path = std::env::temp_dir().join("hntui-test-config.toml");
        let config = match summarize {
            Some(summarize) => Config::for_test_with_summarize(path, summarize),
            None => Config::for_test(path),
        };
        let summarizer = Summarizer::new(None, None, reqwest::Client::new());
        let app = App::new(
            cli(),
            sources,
            tx,
            None,
            config,
            summarizer,
            test_article_fetcher(),
        )
        .with_clipboard(clipboard.clone())
        .with_paste_service(pastes.clone());
        Self { app, clipboard, rx }
    }

    /// Press `H` and let the Paste come back, as the run loop would.
    pub(super) async fn hand_off(&mut self) {
        self.app.handle_action(Action::Handoff);
        self.settle().await;
    }

    pub(super) async fn settle(&mut self) {
        while self.app.tasks.is_running(TaskTarget::Handoff) {
            let event = self.rx.recv().await.expect("handoff event");
            self.app.handle_app_event(event);
        }
    }

    pub(super) fn status(&self) -> &HandoffStatus {
        self.app
            .handoff_status
            .as_ref()
            .expect("a handoff reports itself")
    }
}

pub(super) fn with_done_summary(app: &mut App, story: &Story) {
    app.current_story = Some(story.clone());
    app.summary_overlay.begin(story, 2);
    app.summary_overlay.handle_event(SummaryEvent::Started {
        model: "fake/model".to_string(),
        resolved_model: None,
    });
    app.summary_overlay.handle_event(SummaryEvent::Chunk {
        content: "Widgets are contentious.".to_string(),
        reasoning: String::new(),
    });
    app.summary_overlay
        .handle_event(SummaryEvent::Complete { stats: None });
}

#[tokio::test]
async fn h_publishes_the_summary_and_copies_the_instruction_line() {
    let pastes = Arc::new(RecordingPasteService::default());
    let mut harness = Harness::new(pastes.clone());
    let item = story(42);
    with_done_summary(&mut harness.app, &item);

    harness.hand_off().await;

    let (filename, content) = pastes
        .requests()
        .first()
        .cloned()
        .expect("a paste was created");
    assert_eq!(filename, "hn-42.md");
    assert!(content.starts_with("---\ntitle: \"story 42\""), "{content}");
    assert!(content.contains("model: fake/model"), "{content}");
    assert!(
        content.ends_with("## Summary\n\nWidgets are contentious.\n"),
        "{content}"
    );
    assert_eq!(
        harness.clipboard.last_copied().as_deref(),
        Some(
            "Read the HN thread handoff at https://paste.example/raw/abc123?token=t \
             (story, article, comments, AI summary), then help me with: "
        )
    );
}

#[tokio::test]
async fn a_finished_handoff_reports_the_url_it_copied() {
    let pastes = Arc::new(RecordingPasteService::default());
    let mut harness = Harness::new(pastes);
    let item = story(42);
    with_done_summary(&mut harness.app, &item);

    harness.hand_off().await;

    match harness.status() {
        HandoffStatus::Done {
            raw_url,
            clipboard_error,
        } => {
            assert_eq!(raw_url, "https://paste.example/raw/abc123?token=t");
            assert_eq!(clipboard_error.as_deref(), None);
        }
        other => panic!("expected a finished handoff, got {other:?}"),
    }
}

#[tokio::test]
async fn a_clipboard_that_refuses_the_line_still_leaves_the_url_on_screen() {
    let pastes = Arc::new(RecordingPasteService::default());
    let clipboard = Arc::new(RecordingClipboard::failing("no clipboard here"));
    let mut harness = Harness::with_clipboard(pastes, clipboard);
    let item = story(42);
    with_done_summary(&mut harness.app, &item);

    harness.hand_off().await;

    match harness.status() {
        HandoffStatus::Done {
            raw_url,
            clipboard_error,
        } => {
            assert_eq!(raw_url, "https://paste.example/raw/abc123?token=t");
            assert_eq!(clipboard_error.as_deref(), Some("no clipboard here"));
        }
        other => panic!("expected a finished handoff, got {other:?}"),
    }
}

#[tokio::test]
async fn a_paste_service_failure_is_reported_and_nothing_is_copied() {
    let pastes = Arc::new(RecordingPasteService::failing("service unavailable"));
    let mut harness = Harness::new(pastes);
    let item = story(42);
    with_done_summary(&mut harness.app, &item);

    harness.hand_off().await;

    match harness.status() {
        HandoffStatus::Failed(message) => assert_eq!(message, "service unavailable"),
        other => panic!("expected a failed handoff, got {other:?}"),
    }
    assert!(harness.clipboard.copied().is_empty());
}

#[test]
fn a_streaming_summary_is_refused_rather_than_handed_off_half_written() {
    let pastes = Arc::new(RecordingPasteService::default());
    let mut harness = Harness::new(pastes.clone());
    let item = story(42);
    harness.app.current_story = Some(item.clone());
    harness.app.summary_overlay.begin(&item, 2);
    harness
        .app
        .summary_overlay
        .handle_event(SummaryEvent::Chunk {
            content: "half a".to_string(),
            reasoning: String::new(),
        });

    harness.app.handle_action(Action::Handoff);

    match harness.status() {
        HandoffStatus::Failed(message) => assert_eq!(message, "summary still streaming"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(pastes.requests().is_empty());
}

#[tokio::test]
async fn a_second_h_while_one_is_in_flight_is_refused() {
    let pastes = Arc::new(RecordingPasteService::default());
    let mut harness = Harness::new(pastes.clone());
    let item = story(42);
    with_done_summary(&mut harness.app, &item);

    harness.app.handle_action(Action::Handoff);
    harness.app.handle_action(Action::Handoff);

    match harness.status() {
        HandoffStatus::Failed(message) => assert_eq!(message, "already in progress"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    harness.settle().await;
    assert_eq!(pastes.requests().len(), 1);
}

#[tokio::test]
async fn closing_the_overlay_mid_flight_does_not_cancel_the_handoff() {
    let pastes = Arc::new(RecordingPasteService::default());
    let mut harness = Harness::new(pastes);
    let item = story(42);
    with_done_summary(&mut harness.app, &item);

    harness.app.handle_action(Action::Handoff);
    harness
        .app
        .handle_action(Action::Summary(crate::input::SummaryAction::Dismiss));
    harness.settle().await;

    assert!(matches!(harness.status(), HandoffStatus::Done { .. }));
    assert!(harness.clipboard.last_copied().is_some());
}

#[tokio::test]
async fn a_settled_handoff_line_clears_when_the_user_moves_on() {
    let pastes = Arc::new(RecordingPasteService::default());
    let mut harness = Harness::new(pastes);
    let item = story(42);
    with_done_summary(&mut harness.app, &item);
    harness.hand_off().await;

    harness
        .app
        .handle_action(Action::Summary(crate::input::SummaryAction::ScrollDown(1)));

    assert!(harness.app.handoff_status.is_none());
}

#[tokio::test]
async fn an_in_flight_handoff_keeps_reporting_while_the_user_scrolls() {
    let pastes = Arc::new(RecordingPasteService::default());
    let mut harness = Harness::new(pastes);
    let item = story(42);
    with_done_summary(&mut harness.app, &item);

    harness.app.handle_action(Action::Handoff);
    harness
        .app
        .handle_action(Action::Summary(crate::input::SummaryAction::ScrollDown(1)));

    assert!(matches!(harness.status(), HandoffStatus::InFlight));
}

#[test]
fn with_nothing_loaded_there_is_nothing_to_hand_off() {
    let pastes = Arc::new(RecordingPasteService::default());
    let mut harness = Harness::new(pastes.clone());
    harness.app.current_story = Some(story(42));
    harness.app.view = View::Comments;

    harness.app.handle_action(Action::Handoff);

    match harness.status() {
        HandoffStatus::Failed(message) => assert_eq!(message, "nothing loaded to hand off"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(pastes.requests().is_empty());
}

fn with_article(app: &mut App, story: &Story, content: &str) {
    app.articles.insert(
        story.id,
        crate::article::Article {
            title: None,
            content: content.to_string(),
            effective_url: None,
        },
    );
}

fn with_comments(app: &mut App, story: &Story, comments: Vec<crate::api::CommentNode>) {
    app.apply_comments_for_story(
        story.clone(),
        crate::api::StoryThread::from_comments(comments),
        true,
    );
}

#[tokio::test]
async fn a_handoff_carries_the_article_and_the_comments_that_are_loaded() {
    let pastes = Arc::new(RecordingPasteService::default());
    let mut harness = Harness::new(pastes.clone());
    let item = story(42);
    with_comments(&mut harness.app, &item, vec![comment(11)]);
    with_article(&mut harness.app, &item, "The article text.");

    harness.hand_off().await;

    let content = pastes.last_content().expect("a paste was created");
    assert!(
        content.contains("\n## Article\n\nThe article text.\n"),
        "{content}"
    );
    assert!(
        content.ends_with("## Comments\n\nbob: hello\n"),
        "{content}"
    );
}

#[tokio::test]
async fn the_comments_are_rendered_exactly_as_the_summarizer_renders_them() {
    let pastes = Arc::new(RecordingPasteService::default());
    let mut harness = Harness::new(pastes.clone());
    let item = story(42);
    let mut child = comment(12);
    child.comment.depth = 1;
    child.comment.by = Some("carol".to_string());
    with_comments(&mut harness.app, &item, vec![comment(11), child]);

    harness.hand_off().await;

    let content = pastes.last_content().expect("a paste was created");
    let expected = crate::summarizer::comments_as_thread(&harness.app.comment_list, 200);
    assert!(
        content.ends_with(&format!("## Comments\n\n{}\n", expected.trim_end())),
        "{content}"
    );
    assert!(content.contains("  carol: hello"), "{content}");
}

#[tokio::test]
async fn a_long_article_is_cut_where_the_summarizer_cuts_it() {
    let pastes = Arc::new(RecordingPasteService::default());
    let mut harness = Harness::new(pastes.clone());
    let item = story(42);
    harness.app.view = View::Comments;
    harness.app.current_story = Some(item.clone());
    with_article(&mut harness.app, &item, &"x".repeat(30_000));

    harness.hand_off().await;

    let content = pastes.last_content().expect("a paste was created");
    let expected = crate::summarizer::truncated_article(&"x".repeat(30_000), 20_000);
    assert!(
        content.ends_with(&format!("## Article\n\n{}\n", expected.trim_end())),
        "cut short"
    );
    assert!(content.contains("…[truncated]"), "truncation is announced");
}

#[tokio::test]
async fn an_article_read_with_v_is_carried_even_when_the_summarizer_would_skip_it() {
    let pastes = Arc::new(RecordingPasteService::default());
    let mut harness = Harness::with_summarize(
        pastes.clone(),
        SummarizeConfig {
            model: "fake/model".to_string(),
            api_key: None,
            base_url: None,
            max_comments: 200,
            include_article: false,
            max_article_chars: 20_000,
            system_prompt: "Summarize".to_string(),
        },
    );
    let item = story(42);
    harness.app.view = View::Comments;
    harness.app.current_story = Some(item.clone());
    with_article(&mut harness.app, &item, "The article text.");

    harness.hand_off().await;

    let content = pastes.last_content().expect("a paste was created");
    assert!(content.contains("## Article"), "{content}");
}

#[tokio::test]
async fn comments_loaded_for_another_story_are_not_carried() {
    let pastes = Arc::new(RecordingPasteService::default());
    let mut harness = Harness::new(pastes.clone());
    let other = story(7);
    with_comments(&mut harness.app, &other, vec![comment(11)]);
    let item = story(42);
    harness.app.stories = vec![item.clone()];
    with_article(&mut harness.app, &item, "The article text.");
    harness.app.article_overlay.show(
        &item,
        crate::article::Article {
            title: None,
            content: "The article text.".to_string(),
            effective_url: None,
        },
    );

    harness.hand_off().await;

    let content = pastes.last_content().expect("a paste was created");
    assert!(!content.contains("## Comments"), "{content}");
    assert!(content.contains("## Article"), "{content}");
}

#[tokio::test]
async fn an_article_still_fetching_is_refused_rather_than_waited_on() {
    let pastes = Arc::new(RecordingPasteService::default());
    let mut harness = Harness::new(pastes.clone());
    let item = story(42);
    with_comments(&mut harness.app, &item, vec![comment(11)]);
    harness.app.handle_action(Action::ViewArticle);

    harness.app.handle_action(Action::Handoff);

    match harness.status() {
        HandoffStatus::Failed(message) => assert_eq!(message, "article still fetching"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(pastes.requests().is_empty());
}

#[tokio::test]
async fn h_in_the_article_overlay_hands_off_the_story_it_is_showing() {
    let pastes = Arc::new(RecordingPasteService::default());
    let mut harness = Harness::new(pastes.clone());
    let item = story(42);
    with_comments(&mut harness.app, &item, vec![comment(11)]);
    with_article(&mut harness.app, &item, "The article text.");
    harness.app.handle_action(Action::ViewArticle);
    assert_eq!(
        harness.app.input_layer(),
        crate::input::InputLayer::Article,
        "the article overlay should have the input"
    );

    harness.hand_off().await;

    let (filename, content) = pastes
        .requests()
        .first()
        .cloned()
        .expect("a paste was created");
    assert_eq!(filename, "hn-42.md");
    assert!(
        content.contains("## Article\n\nThe article text.\n"),
        "{content}"
    );
    assert!(content.contains("## Comments\n\nbob: hello"), "{content}");
    assert!(!content.contains("## Summary"), "{content}");
}

#[tokio::test]
async fn h_in_the_comments_view_hands_off_the_discussion() {
    let pastes = Arc::new(RecordingPasteService::default());
    let mut harness = Harness::new(pastes.clone());
    let item = story(42);
    with_comments(&mut harness.app, &item, vec![comment(11)]);
    assert_eq!(harness.app.view, View::Comments);

    harness.hand_off().await;

    let content = pastes.last_content().expect("a paste was created");
    assert!(content.starts_with("---\ntitle: \"story 42\""), "{content}");
    assert!(
        content.ends_with("## Comments\n\nbob: hello\n"),
        "{content}"
    );
    assert!(!content.contains("## Article"), "{content}");
    assert!(!content.contains("model:"), "{content}");
}

#[tokio::test]
async fn a_dismissed_summary_is_not_carried_by_a_later_handoff() {
    let pastes = Arc::new(RecordingPasteService::default());
    let mut harness = Harness::new(pastes.clone());
    let item = story(42);
    with_comments(&mut harness.app, &item, vec![comment(11)]);
    with_done_summary(&mut harness.app, &item);
    harness
        .app
        .handle_action(Action::Summary(crate::input::SummaryAction::Dismiss));

    harness.hand_off().await;

    let content = pastes.last_content().expect("a paste was created");
    assert!(!content.contains("## Summary"), "{content}");
    assert!(content.contains("## Comments"), "{content}");
}

#[tokio::test]
async fn the_handoff_line_is_not_hidden_by_an_error_from_earlier() {
    let pastes = Arc::new(RecordingPasteService::default());
    let mut harness = Harness::new(pastes);
    let item = story(42);
    with_comments(&mut harness.app, &item, vec![comment(11)]);
    harness.app.last_error = Some("something failed earlier".to_string());

    harness.hand_off().await;

    // The footer picks one line; the Handoff has to win it, or the URL the
    // user just asked for never reaches the screen.
    harness
        .app
        .prepare_frame(ratatui::layout::Rect::new(0, 0, 120, 24));
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 24)).expect("test terminal");
    terminal
        .draw(|frame| crate::ui::render(frame, &harness.app))
        .expect("draw");
    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(
        rendered.contains("handoff → https://paste.example"),
        "{rendered}"
    );
    assert!(!rendered.contains("something failed earlier"), "{rendered}");
}

#[tokio::test]
async fn an_ask_hn_carries_the_question_its_replies_answer() {
    let pastes = Arc::new(RecordingPasteService::default());
    let mut harness = Harness::new(pastes.clone());
    let mut item = story(42);
    item.url = None;
    item.text = Some("<p>How do you test TUIs?".to_string());
    with_comments(&mut harness.app, &item, vec![comment(11)]);

    harness.hand_off().await;

    let content = pastes.last_content().expect("a paste was created");
    assert!(content.contains("How do you test TUIs?"), "{content}");
    assert!(content.contains("## Comments"), "{content}");
}

#[tokio::test]
async fn the_front_matter_never_claims_more_comments_than_it_carries() {
    let pastes = Arc::new(RecordingPasteService::default());
    let mut harness = Harness::new(pastes.clone());
    let mut item = story(42);
    item.comment_count = 312;
    with_comments(&mut harness.app, &item, vec![comment(11), comment(12)]);

    harness.hand_off().await;

    let content = pastes.last_content().expect("a paste was created");
    assert!(content.contains("comments: 2\n"), "{content}");
    assert!(content.contains("comments_total: 312\n"), "{content}");
}
