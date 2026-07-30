use super::{LlmSession, Summarizer, SummaryRequest};
use futures::future::BoxFuture;
use futures::StreamExt;
use std::time::{Duration, Instant};

const CONNECTION_TEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConnectionDraft {
    pub(crate) model: String,
    pub(crate) system_prompt: String,
    pub(crate) api_key: Option<String>,
    pub(crate) base_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConnectionTestSuccess {
    pub(crate) model: String,
    pub(crate) ttft: Duration,
}

#[derive(Debug)]
pub(crate) enum ConnectionTestError {
    TimedOut,
    Llm(smolllm::Error),
}

impl ConnectionTestError {
    pub(crate) fn friendly_message(&self) -> String {
        match self {
            Self::TimedOut => "timed out after 15s".to_string(),
            Self::Llm(error) => super::friendly_llm_error(error, Some(CONNECTION_TEST_TIMEOUT)),
        }
    }
}

impl Summarizer {
    pub(crate) fn test_connection(
        &self,
        draft: ConnectionDraft,
    ) -> BoxFuture<'static, Result<ConnectionTestSuccess, ConnectionTestError>> {
        self.test_connection_with_timeout(draft, CONNECTION_TEST_TIMEOUT)
    }

    pub(super) fn test_connection_with_timeout(
        &self,
        draft: ConnectionDraft,
        timeout: Duration,
    ) -> BoxFuture<'static, Result<ConnectionTestSuccess, ConnectionTestError>> {
        let llm = self.stream.clone();
        Box::pin(async move {
            let started = Instant::now();
            let result = tokio::time::timeout(timeout, async move {
                let request = SummaryRequest {
                    model: draft.model,
                    system_prompt: draft.system_prompt,
                    user_prompt: "hi".to_string(),
                    api_key: draft.api_key,
                    base_url: draft.base_url,
                };
                let LlmSession { model, mut chunks } =
                    llm.start(request).await.map_err(ConnectionTestError::Llm)?;

                while let Some(chunk) = chunks.next().await {
                    let chunk = chunk.map_err(ConnectionTestError::Llm)?;
                    if !chunk.content.is_empty() {
                        return Ok(ConnectionTestSuccess {
                            model,
                            ttft: started.elapsed(),
                        });
                    }
                }

                Err(ConnectionTestError::Llm(smolllm::Error::EmptyResponse {
                    model,
                }))
            })
            .await;

            result.unwrap_or(Err(ConnectionTestError::TimedOut))
        })
    }
}
