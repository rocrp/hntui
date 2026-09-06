//! `H`: what reaches the Paste, what reaches the clipboard, and what the
//! footer says about it.

use super::tests::{cli, story, test_article_fetcher};
use super::*;
use crate::api::{InMemorySource, Sources};
use crate::clipboard::RecordingClipboard;
use crate::config::Config;
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
        Self::with_clipboard(pastes, Arc::new(RecordingClipboard::default()))
    }

    pub(super) fn with_clipboard(
        pastes: Arc<RecordingPasteService>,
        clipboard: Arc<RecordingClipboard>,
    ) -> Self {
        let source = Arc::new(InMemorySource::default());
        let sources = Sources::new(source.clone(), source);
        let (tx, rx) = mpsc::unbounded_channel();
        let config = Config::for_test(std::env::temp_dir().join("hntui-test-config.toml"));
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
