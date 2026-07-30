use super::tests::{cli, comment, story, test_article_fetcher};
use super::{App, AppEvent, TaskTarget};
use crate::api::{InMemorySource, Sources, StoryThread};
use crate::config::{Config, SummarizeConfig};
use crate::input::Action;
use crate::summarizer::{
    LlmFuture, LlmSession, LlmStream, Summarizer, SummaryChunk, SummaryRequest,
};
use crate::ui::summary_overlay::SummaryState;
use futures::FutureExt;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Clone, Copy)]
enum FailureStage {
    Initialization,
    MidStream,
}

#[derive(Clone)]
struct HttpFailingStream {
    stage: FailureStage,
    status: u16,
    body: String,
}

impl LlmStream for HttpFailingStream {
    fn start(&self, _request: SummaryRequest) -> LlmFuture {
        let stage = self.stage;
        let status = self.status;
        let body = self.body.clone();
        async move {
            let error = || smolllm::Error::Http {
                status,
                body: body.clone(),
            };
            match stage {
                FailureStage::Initialization => Err(error()),
                FailureStage::MidStream => Ok(LlmSession::for_test(
                    "fake/model",
                    vec![
                        Ok(SummaryChunk {
                            content: "partial".to_string(),
                            reasoning: String::new(),
                        }),
                        Err(error()),
                    ],
                )),
            }
        }
        .boxed()
    }
}

fn app_with_http_failure(
    stage: FailureStage,
    status: u16,
    body: String,
) -> (App, mpsc::UnboundedReceiver<AppEvent>) {
    let item = story(1);
    let source = Arc::new(InMemorySource::default());
    let sources = Sources::new(source.clone(), source);
    let (tx, rx) = mpsc::unbounded_channel();
    let summarize = SummarizeConfig {
        model: "fake/model".to_string(),
        api_key: None,
        base_url: None,
        max_comments: 20,
        include_article: false,
        max_article_chars: 20_000,
        system_prompt: "Summarize".to_string(),
    };
    let config = Config::for_test_with_summarize(
        std::env::temp_dir().join("hntui-summary-friendly-error.toml"),
        summarize.clone(),
    );
    let summarizer = Summarizer::with_stream(
        Some(summarize),
        None,
        Arc::new(HttpFailingStream {
            stage,
            status,
            body,
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
    app.apply_comments_for_story(item, StoryThread::from_comments(vec![comment(11)]), false);
    (app, rx)
}

async fn summarize_to_failure(app: &mut App, rx: &mut mpsc::UnboundedReceiver<AppEvent>) {
    app.handle_action(Action::Summarize);
    while app.tasks.is_running(TaskTarget::Summary) {
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("summary event timed out")
            .expect("app event channel closed");
        app.handle_app_event(event);
    }
}

#[tokio::test]
async fn initialization_http_401_reaches_summary_overlay_as_bounded_friendly_text() {
    let (mut app, mut rx) =
        app_with_http_failure(FailureStage::Initialization, 401, "x".repeat(500));

    summarize_to_failure(&mut app, &mut rx).await;

    assert_eq!(app.summary_overlay.state(), SummaryState::Error);
    assert_eq!(
        app.summary_overlay.error_message(),
        Some(format!("check API key · {}…", "x".repeat(120)).as_str())
    );
    assert!(!app
        .summary_overlay
        .error_message()
        .expect("summary error")
        .contains("HTTP error 401"));
}

#[tokio::test]
async fn mid_stream_http_error_uses_the_same_friendly_summary_overlay_path() {
    let (mut app, mut rx) = app_with_http_failure(
        FailureStage::MidStream,
        403,
        "permission denied".to_string(),
    );

    summarize_to_failure(&mut app, &mut rx).await;

    assert_eq!(app.summary_overlay.state(), SummaryState::Error);
    assert_eq!(
        app.summary_overlay.error_message(),
        Some("check API key · permission denied")
    );
}
