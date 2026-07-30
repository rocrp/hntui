use crate::api::types::{Comment, Story};
use crate::config::SummarizeConfig;
use crate::text::hn_html_to_plain;
use anyhow::Result;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use futures::StreamExt;
use std::sync::Arc;

mod connection;
#[cfg(test)]
mod connection_tests;
mod friendly_error;
pub(crate) use connection::{ConnectionDraft, ConnectionTestError, ConnectionTestSuccess};
pub(crate) use friendly_error::friendly_llm_error;

pub(crate) type LlmResult<T> = std::result::Result<T, smolllm::Error>;
pub(crate) type LlmFuture = BoxFuture<'static, LlmResult<LlmSession>>;

pub(crate) trait LlmStream: Send + Sync {
    fn start(&self, request: SummaryRequest) -> LlmFuture;
}

pub(crate) struct LlmSession {
    model: String,
    chunks: BoxStream<'static, LlmResult<SummaryChunk>>,
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
    /// Article text to ground the summary in. `None` when the toggle is off,
    /// the story has nothing to fetch, or the fetch failed.
    pub article: Option<String>,
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
            let article = input.article.as_deref().filter(|text| !text.trim().is_empty());
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
                    yield Err(anyhow::Error::new(error));
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
                        yield Err(anyhow::Error::new(error));
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

            let stream = builder.await?;
            let model = stream.model().to_string();
            let chunks = stream.map(|chunk| {
                chunk.map(|chunk| SummaryChunk {
                    content: chunk.content,
                    reasoning: chunk.reasoning,
                })
            });
            Ok(LlmSession {
                model,
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
        prompt.push_str(&truncate_article(article, max_article_chars));
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
    for comment in comments.iter().take(max_comments) {
        let author = comment.by.as_deref().unwrap_or("[anon]");
        let indent = "  ".repeat(comment.depth);
        let text = hn_html_to_plain(&comment.text);
        prompt.push_str(&format!("{indent}{author}: {text}\n\n"));
    }
    prompt
}

/// Head-truncate on a char boundary; the lead of an article carries the thesis.
fn truncate_article(article: &str, max_chars: usize) -> String {
    let mut truncated: String = article.chars().take(max_chars).collect();
    if truncated.chars().count() < article.chars().count() {
        truncated.push_str("\n\n…[truncated]");
    }
    truncated
}

#[cfg(test)]
mod tests;
