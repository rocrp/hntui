use crate::api::types::{Comment, Story};
use crate::config::SummarizeConfig;
use anyhow::Result;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use futures::StreamExt;
use std::sync::Arc;

mod connection;
#[cfg(test)]
mod connection_tests;
mod friendly_error;
pub(crate) mod material;
pub(crate) use connection::{ConnectionDraft, ConnectionTestError, ConnectionTestSuccess};
pub(crate) use friendly_error::friendly_llm_error;
pub(crate) use material::{comments_as_thread, truncated_article};

pub(crate) type LlmResult<T> = std::result::Result<T, smolllm::Error>;
pub(crate) type LlmFuture = BoxFuture<'static, LlmResult<LlmSession>>;

pub(crate) trait LlmStream: Send + Sync {
    fn start(&self, request: SummaryRequest) -> LlmFuture;
}

/// What one completed LLM call cost and whether it finished. Filled in once the
/// stream is exhausted, so it is read after the last chunk, never before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SummaryStats {
    pub duration: std::time::Duration,
    pub ttft: Option<std::time::Duration>,
    pub input_tokens: usize,
    pub output_tokens: usize,
    /// True when a token count is a chars/4 guess rather than the provider's own.
    pub estimated: bool,
    /// True when the answer was cut short: the model hit its output cap, or the
    /// stream ended without its terminal frame.
    pub truncated: bool,
}

/// Written by the chunk stream when it ends; read by the summarize loop after.
pub(crate) type SharedStats = Arc<std::sync::Mutex<Option<SummaryStats>>>;

pub(crate) struct LlmSession {
    model: String,
    /// What the server said is answering, when it said anything. Known by the
    /// time the session exists: the library waits for the first chunk before
    /// handing the stream over, and that frame names the model.
    resolved_model: Option<String>,
    stats: SharedStats,
    chunks: BoxStream<'static, LlmResult<SummaryChunk>>,
}

#[cfg(test)]
impl LlmSession {
    pub(crate) fn for_test(model: &str, chunks: Vec<LlmResult<SummaryChunk>>) -> Self {
        Self {
            model: model.to_string(),
            resolved_model: None,
            stats: SharedStats::default(),
            chunks: Box::pin(futures::stream::iter(chunks)),
        }
    }

    pub(crate) fn for_test_resolving_to(
        model: &str,
        resolved_model: &str,
        chunks: Vec<LlmResult<SummaryChunk>>,
    ) -> Self {
        Self {
            resolved_model: Some(resolved_model.to_string()),
            ..Self::for_test(model, chunks)
        }
    }
}

/// The Article text as the prompt will see it: whitespace-only extraction is
/// nothing to ground a summary in. The overlay reports inclusion from this same
/// rule, so its stats line cannot claim an article the prompt never carried.
pub(crate) fn usable_article(article: Option<&str>) -> Option<&str> {
    article.filter(|text| !text.trim().is_empty())
}

pub(crate) struct SummaryRequest {
    model: String,
    system_prompt: String,
    user_prompt: String,
    api_key: Option<String>,
    base_url: Option<String>,
}

#[cfg(test)]
impl SummaryRequest {
    pub(crate) fn user_prompt(&self) -> &str {
        &self.user_prompt
    }
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
    /// Article text to ground the summary in. `None` when the toggle is off,
    /// the story has nothing to fetch, or the fetch failed.
    pub article: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummaryEvent {
    Started {
        model: String,
        /// The ResolvedModel, when the backend reported one that differs from
        /// what was asked for.
        resolved_model: Option<String>,
    },
    Chunk {
        content: String,
        reasoning: String,
    },
    Complete {
        /// What the call cost, when the backend reported enough to say. None
        /// from a backend that reports nothing.
        stats: Option<SummaryStats>,
    },
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
                    "LLM not configured. Press , to edit the config or set HNTUI_LLM_API_KEY"
                ));
                return;
            };
            let article = usable_article(input.article.as_deref());
            if input.comments.is_empty() && article.is_none() {
                yield Err(anyhow::anyhow!("No comments to summarize"));
                return;
            }

            let request = SummaryRequest {
                model: config.model,
                system_prompt: config.system_prompt,
                user_prompt: build_prompt(
                    &input.story,
                    &input.comments,
                    article,
                    config.max_comments,
                    config.max_article_chars,
                ),
                api_key: api_key_override,
                base_url: config.base_url,
            };
            let mut session = match llm.start(request).await {
                Ok(session) => session,
                Err(error) => {
                    yield Err(summary_llm_error(error));
                    return;
                }
            };
            yield Ok(SummaryEvent::Started {
                model: session.model,
                resolved_model: session.resolved_model,
            });

            while let Some(chunk) = session.chunks.next().await {
                match chunk {
                    Ok(chunk) if chunk.content.is_empty() && chunk.reasoning.is_empty() => {}
                    Ok(chunk) => yield Ok(SummaryEvent::Chunk {
                        content: chunk.content,
                        reasoning: chunk.reasoning,
                    }),
                    Err(error) => {
                        yield Err(summary_llm_error(error));
                        return;
                    }
                }
            }
            let stats = session
                .stats
                .lock()
                .ok()
                .and_then(|stats| *stats);
            yield Ok(SummaryEvent::Complete { stats });
        })
    }
}

fn summary_llm_error(error: smolllm::Error) -> anyhow::Error {
    anyhow::Error::msg(friendly_llm_error(&error, None))
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

            let mut stream = builder.await?;
            let model = stream.model().to_string();
            let resolved_model = stream.resolved_model();
            let stats: SharedStats = SharedStats::default();
            let sink = Arc::clone(&stats);
            // The stream owns the response, so what it cost can be read from it
            // the moment the last chunk has gone by.
            let chunks = async_stream::stream! {
                while let Some(chunk) = stream.next().await {
                    match chunk {
                        Ok(chunk) => yield Ok(SummaryChunk {
                            content: chunk.content,
                            reasoning: chunk.reasoning,
                        }),
                        Err(error) => {
                            yield Err(error);
                            return;
                        }
                    }
                }
                let usage = stream.usage();
                if let Ok(mut sink) = sink.lock() {
                    *sink = Some(SummaryStats {
                        duration: usage.duration,
                        ttft: usage.ttft,
                        input_tokens: usage.input_tokens,
                        output_tokens: usage.output_tokens,
                        estimated: usage.estimated,
                        truncated: stream.truncated(),
                    });
                }
            };
            Ok(LlmSession {
                model,
                resolved_model,
                stats,
                chunks: Box::pin(chunks),
            })
        })
    }
}

fn build_prompt(
    story: &Story,
    comments: &[Comment],
    article: Option<&str>,
    max_comments: usize,
    max_article_chars: usize,
) -> String {
    let mut prompt = format!("# {}\n\n", story.title);

    if let Some(article) = article {
        prompt.push_str("## Article\n\n");
        prompt.push_str(&truncated_article(article, max_article_chars));
        prompt.push_str("\n\n");
    }

    if comments.is_empty() {
        return prompt;
    }

    // Only label the comments once there is another section to tell them from;
    // an article-less prompt stays byte-identical to the pre-Article shape.
    if article.is_some() {
        prompt.push_str("## Comments\n\n");
    }
    prompt.push_str(&comments_as_thread(comments, max_comments));
    prompt
}

#[cfg(test)]
mod tests;
