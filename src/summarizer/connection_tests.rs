use super::*;
use futures::FutureExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

struct DropSignal(Option<oneshot::Sender<()>>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

#[derive(Clone)]
struct CapturingStream {
    request: Arc<Mutex<Option<SummaryRequest>>>,
    dropped: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

#[derive(Clone)]
struct ReasoningOnlyStream {
    dropped: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

impl LlmStream for ReasoningOnlyStream {
    fn start(&self, _request: SummaryRequest) -> LlmFuture {
        let dropped = self
            .dropped
            .lock()
            .expect("drop lock poisoned")
            .take()
            .expect("one connection stream");
        async move {
            let chunks = async_stream::stream! {
                let _drop_signal = DropSignal(Some(dropped));
                yield Ok(SummaryChunk {
                    content: String::new(),
                    reasoning: "still thinking".to_string(),
                });
                futures::future::pending::<()>().await;
            };
            Ok(LlmSession {
                model: "fake/model".to_string(),
                chunks: Box::pin(chunks),
            })
        }
        .boxed()
    }
}

impl LlmStream for CapturingStream {
    fn start(&self, request: SummaryRequest) -> LlmFuture {
        *self.request.lock().expect("request lock poisoned") = Some(request);
        let dropped = self
            .dropped
            .lock()
            .expect("drop lock poisoned")
            .take()
            .expect("one connection stream");
        async move {
            let chunks = async_stream::stream! {
                let _drop_signal = DropSignal(Some(dropped));
                yield Ok(SummaryChunk {
                    content: String::new(),
                    reasoning: "thinking".to_string(),
                });
                yield Ok(SummaryChunk {
                    content: "hello".to_string(),
                    reasoning: String::new(),
                });
                futures::future::pending::<()>().await;
            };
            Ok(LlmSession {
                model: "fallback/served-model".to_string(),
                chunks: Box::pin(chunks),
            })
        }
        .boxed()
    }
}

#[tokio::test]
async fn connection_test_uses_the_draft_request_and_drops_after_first_content() {
    let request = Arc::new(Mutex::new(None));
    let (dropped_tx, dropped_rx) = oneshot::channel();
    let stream = CapturingStream {
        request: request.clone(),
        dropped: Arc::new(Mutex::new(Some(dropped_tx))),
    };
    let summarizer = Summarizer::with_stream(None, None, Arc::new(stream));
    let draft = ConnectionDraft {
        model: "missing/model, fallback/model".to_string(),
        system_prompt: "Draft instructions".to_string(),
        api_key: Some("draft-key".to_string()),
        base_url: Some("https://draft.example/v1".to_string()),
    };

    let success = summarizer
        .test_connection_with_timeout(draft, Duration::from_secs(1))
        .await
        .expect("connection succeeds");

    assert_eq!(success.model, "fallback/served-model");
    let captured = request
        .lock()
        .expect("request lock poisoned")
        .take()
        .expect("captured request");
    assert_eq!(captured.model, "missing/model, fallback/model");
    assert_eq!(captured.system_prompt, "Draft instructions");
    assert_eq!(captured.user_prompt, "hi");
    assert_eq!(captured.api_key.as_deref(), Some("draft-key"));
    assert_eq!(
        captured.base_url.as_deref(),
        Some("https://draft.example/v1")
    );
    tokio::time::timeout(Duration::from_secs(1), dropped_rx)
        .await
        .expect("stream was not dropped")
        .expect("drop signal closed");
}

#[test]
fn connection_errors_stay_typed_until_friendly_presentation() {
    assert_eq!(
        ConnectionTestError::TimedOut.friendly_message(),
        "timed out after 15s"
    );
    assert_eq!(
        ConnectionTestError::Llm(smolllm::Error::Http {
            status: 404,
            body: "not found".to_string(),
        })
        .friendly_message(),
        "endpoint path wrong? if your URL is already the full endpoint, end it with `#`"
    );
}

#[tokio::test]
async fn reasoning_only_does_not_satisfy_ttft_and_the_whole_test_times_out() {
    let (dropped_tx, dropped_rx) = oneshot::channel();
    let summarizer = Summarizer::with_stream(
        None,
        None,
        Arc::new(ReasoningOnlyStream {
            dropped: Arc::new(Mutex::new(Some(dropped_tx))),
        }),
    );
    let result = summarizer
        .test_connection_with_timeout(
            ConnectionDraft {
                model: "fake/model".to_string(),
                system_prompt: "test".to_string(),
                api_key: None,
                base_url: None,
            },
            Duration::from_millis(10),
        )
        .await;

    assert!(matches!(result, Err(ConnectionTestError::TimedOut)));
    tokio::time::timeout(Duration::from_secs(1), dropped_rx)
        .await
        .expect("timed-out stream was not dropped")
        .expect("drop signal closed");
}
