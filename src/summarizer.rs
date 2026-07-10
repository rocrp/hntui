use crate::api::types::{Comment, Story};
use crate::config::SummarizeConfig;
use crate::text::hn_html_to_plain;
use anyhow::{Context, Result};
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use futures::StreamExt;
use std::sync::Arc;

pub(crate) type LlmFuture = BoxFuture<'static, Result<LlmSession>>;

pub(crate) trait LlmStream: Send + Sync {
    fn start(&self, request: SummaryRequest) -> LlmFuture;
}

pub(crate) struct LlmSession {
    model: String,
    chunks: BoxStream<'static, Result<SummaryChunk>>,
}

pub(crate) struct SummaryRequest {
    model: String,
    system_prompt: String,
    user_prompt: String,
    api_key: Option<String>,
    base_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SummaryChunk {
    pub content: String,
    pub reasoning: String,
}

#[derive(Debug, Clone)]
pub struct SummaryInput {
    pub story: Story,
    pub comments: Vec<Comment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummaryEvent {
    Started { model: String },
    Chunk { content: String, reasoning: String },
    Complete,
}

#[derive(Clone)]
pub struct Summarizer {
    config: Option<SummarizeConfig>,
    api_key_override: Option<String>,
    stream: Arc<dyn LlmStream>,
}

impl Summarizer {
    pub fn new(
        config: Option<SummarizeConfig>,
        api_key_override: Option<String>,
        http: reqwest::Client,
    ) -> Self {
        Self {
            config,
            api_key_override,
            stream: Arc::new(SmolLlmStream { http }),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_stream(
        config: Option<SummarizeConfig>,
        api_key_override: Option<String>,
        stream: Arc<dyn LlmStream>,
    ) -> Self {
        Self {
            config,
            api_key_override,
            stream,
        }
    }

    pub fn update_config(
        &mut self,
        config: Option<SummarizeConfig>,
        api_key_override: Option<String>,
    ) {
        self.config = config;
        self.api_key_override = api_key_override;
    }

    pub fn summarize(&self, input: SummaryInput) -> BoxStream<'static, Result<SummaryEvent>> {
        let config = self.config.clone();
        let api_key_override = self.api_key_override.clone();
        let llm = self.stream.clone();
        Box::pin(async_stream::stream! {
            let Some(config) = config else {
                yield Err(anyhow::anyhow!(
                    "LLM not configured. Press , for settings or set HNTUI_LLM_API_KEY"
                ));
                return;
            };
            if input.comments.is_empty() {
                yield Err(anyhow::anyhow!("No comments to summarize"));
                return;
            }

            let request = SummaryRequest {
                model: config.model,
                system_prompt: config.system_prompt,
                user_prompt: build_prompt(&input.story, &input.comments, config.max_comments),
                api_key: api_key_override,
                base_url: config.base_url,
            };
            let mut session = match llm.start(request).await {
                Ok(session) => session,
                Err(error) => {
                    yield Err(error);
                    return;
                }
            };
            yield Ok(SummaryEvent::Started {
                model: session.model,
            });

            while let Some(chunk) = session.chunks.next().await {
                match chunk {
                    Ok(chunk) if chunk.content.is_empty() && chunk.reasoning.is_empty() => {}
                    Ok(chunk) => yield Ok(SummaryEvent::Chunk {
                        content: chunk.content,
                        reasoning: chunk.reasoning,
                    }),
                    Err(error) => {
                        yield Err(error);
                        return;
                    }
                }
            }
            yield Ok(SummaryEvent::Complete);
        })
    }
}

#[derive(Clone)]
struct SmolLlmStream {
    http: reqwest::Client,
}

impl LlmStream for SmolLlmStream {
    fn start(&self, request: SummaryRequest) -> LlmFuture {
        let http = self.http.clone();
        Box::pin(async move {
            let mut builder = smolllm::stream(request.user_prompt)
                .model(&request.model)
                .system_prompt(&request.system_prompt)
                .http_client(http);
            if let Some(api_key) = request.api_key {
                builder = builder.api_key(api_key);
            }
            if let Some(base_url) = request.base_url {
                builder = builder.base_url(base_url);
            }

            let stream = builder.await.context("failed to initialize stream")?;
            let model = stream.model().to_string();
            let chunks = stream.map(|chunk| {
                chunk
                    .map(|chunk| SummaryChunk {
                        content: chunk.content,
                        reasoning: chunk.reasoning,
                    })
                    .map_err(anyhow::Error::from)
            });
            Ok(LlmSession {
                model,
                chunks: Box::pin(chunks),
            })
        })
    }
}

fn build_prompt(story: &Story, comments: &[Comment], max_comments: usize) -> String {
    let mut prompt = format!("# {}\n\n", story.title);
    for comment in comments.iter().take(max_comments) {
        let author = comment.by.as_deref().unwrap_or("[anon]");
        let indent = "  ".repeat(comment.depth);
        let text = hn_html_to_plain(&comment.text);
        prompt.push_str(&format!("{indent}{author}: {text}\n\n"));
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::{Comment, Story};
    use crate::config::SummarizeConfig;
    use futures::{stream, FutureExt};
    use std::sync::Arc;

    #[derive(Clone)]
    struct FakeLlmStream;

    impl LlmStream for FakeLlmStream {
        fn start(&self, _request: SummaryRequest) -> LlmFuture {
            async move {
                Ok(LlmSession {
                    model: "fake/model".to_string(),
                    chunks: Box::pin(stream::iter(vec![
                        Ok(SummaryChunk {
                            content: String::new(),
                            reasoning: "thinking".to_string(),
                        }),
                        Ok(SummaryChunk {
                            content: "answer".to_string(),
                            reasoning: String::new(),
                        }),
                    ])),
                })
            }
            .boxed()
        }
    }

    #[derive(Clone)]
    struct FailingLlmStream {
        fail_during_stream: bool,
    }

    impl LlmStream for FailingLlmStream {
        fn start(&self, _request: SummaryRequest) -> LlmFuture {
            let fail_during_stream = self.fail_during_stream;
            async move {
                if !fail_during_stream {
                    anyhow::bail!("initialization failed");
                }
                Ok(LlmSession {
                    model: "fake/model".to_string(),
                    chunks: Box::pin(stream::iter(vec![
                        Ok(SummaryChunk {
                            content: "partial".to_string(),
                            reasoning: String::new(),
                        }),
                        Err(anyhow::anyhow!("stream failed")),
                    ])),
                })
            }
            .boxed()
        }
    }

    fn input() -> SummaryInput {
        SummaryInput {
            story: Story {
                id: 1,
                title: "Story".to_string(),
                url: None,
                score: 10,
                by: "alice".to_string(),
                time: 1,
                comment_count: 1,
                kids: vec![2],
            },
            comments: vec![Comment {
                id: 2,
                by: Some("bob".to_string()),
                time: Some(1),
                text: "hello".to_string(),
                kids: vec![],
                depth: 0,
                collapsed: false,
                children_loaded: true,
                children_loading: false,
            }],
        }
    }

    fn config() -> SummarizeConfig {
        SummarizeConfig {
            model: "fake/model".to_string(),
            api_key: None,
            base_url: None,
            max_comments: 20,
            system_prompt: "Summarize".to_string(),
        }
    }

    #[tokio::test]
    async fn fake_stream_emits_started_chunks_and_complete_in_order() {
        let summarizer = Summarizer::with_stream(
            Some(config()),
            Some("test-key".to_string()),
            Arc::new(FakeLlmStream),
        );
        let events = summarizer
            .summarize(input())
            .map(Result::unwrap)
            .collect::<Vec<_>>()
            .await;

        assert_eq!(
            events,
            vec![
                SummaryEvent::Started {
                    model: "fake/model".to_string()
                },
                SummaryEvent::Chunk {
                    content: String::new(),
                    reasoning: "thinking".to_string()
                },
                SummaryEvent::Chunk {
                    content: "answer".to_string(),
                    reasoning: String::new()
                },
                SummaryEvent::Complete,
            ]
        );
    }

    #[tokio::test]
    async fn initialization_error_surfaces_without_complete() {
        let summarizer = Summarizer::with_stream(
            Some(config()),
            None,
            Arc::new(FailingLlmStream {
                fail_during_stream: false,
            }),
        );

        let mut events = summarizer.summarize(input());

        let error = events
            .next()
            .await
            .expect("error event")
            .expect_err("initialization should fail");
        assert_eq!(error.to_string(), "initialization failed");
        assert!(events.next().await.is_none());
    }

    #[tokio::test]
    async fn mid_stream_error_preserves_prior_chunks_without_complete() {
        let summarizer = Summarizer::with_stream(
            Some(config()),
            None,
            Arc::new(FailingLlmStream {
                fail_during_stream: true,
            }),
        );

        let mut events = summarizer.summarize(input());

        assert_eq!(
            events
                .next()
                .await
                .expect("started event")
                .expect("started"),
            SummaryEvent::Started {
                model: "fake/model".to_string()
            }
        );
        assert_eq!(
            events.next().await.expect("chunk event").expect("chunk"),
            SummaryEvent::Chunk {
                content: "partial".to_string(),
                reasoning: String::new()
            }
        );
        let error = events
            .next()
            .await
            .expect("error event")
            .expect_err("stream should fail");
        assert_eq!(error.to_string(), "stream failed");
        assert!(events.next().await.is_none());
    }
}
