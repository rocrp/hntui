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
                return Err(smolllm::Error::Other("initialization failed".to_string()));
            }
            Ok(LlmSession {
                model: "fake/model".to_string(),
                chunks: Box::pin(stream::iter(vec![
                    Ok(SummaryChunk {
                        content: "partial".to_string(),
                        reasoning: String::new(),
                    }),
                    Err(smolllm::Error::Other("stream failed".to_string())),
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
            text: None,
            score: 10,
            by: "alice".to_string(),
            time: 1,
            comment_count: 1,
            kids: vec![2],
        },
        article: None,
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
        include_article: true,
        max_article_chars: 20_000,
        system_prompt: "Summarize".to_string(),
    }
}

fn comment(author: &str, text: &str, depth: usize) -> Comment {
    Comment {
        id: 2,
        by: Some(author.to_string()),
        time: Some(1),
        text: text.to_string(),
        kids: vec![],
        depth,
        collapsed: false,
        children_loaded: true,
        children_loading: false,
    }
}

fn story() -> Story {
    input().story
}

#[test]
fn a_prompt_without_an_article_keeps_the_pre_article_shape() {
    let comments = [comment("bob", "hello", 0), comment("carol", "reply", 1)];
    let prompt = build_prompt(&story(), &comments, None, 20, 20_000);
    assert_eq!(prompt, "# Story\n\nbob: hello\n\n  carol: reply\n\n");
}

#[test]
fn an_article_prompt_labels_all_three_sections() {
    let comments = [comment("bob", "hello", 0)];
    let prompt = build_prompt(&story(), &comments, Some("the body"), 20, 20_000);
    assert_eq!(
        prompt,
        "# Story\n\n## Article\n\nthe body\n\n## Comments\n\nbob: hello\n\n"
    );
}

#[test]
fn an_article_longer_than_the_cap_is_head_truncated_with_a_marker() {
    let prompt = build_prompt(&story(), &[], Some("abcdefghij"), 20, 4);
    assert_eq!(prompt, "# Story\n\n## Article\n\nabcd\n\n…[truncated]\n\n");
}

#[test]
fn truncation_counts_characters_not_bytes() {
    let truncated = truncate_article("世界你好世界", 3);
    assert!(truncated.starts_with("世界你"));
    assert!(truncated.ends_with("…[truncated]"));
}

#[test]
fn an_article_that_fits_carries_no_truncation_marker() {
    let prompt = build_prompt(&story(), &[], Some("short"), 20, 20_000);
    assert_eq!(prompt, "# Story\n\n## Article\n\nshort\n\n");
}

#[test]
fn max_comments_still_caps_the_comment_section() {
    let comments = [
        comment("a", "one", 0),
        comment("b", "two", 0),
        comment("c", "three", 0),
    ];
    let prompt = build_prompt(&story(), &comments, None, 2, 20_000);
    assert!(prompt.contains("a: one"));
    assert!(prompt.contains("b: two"));
    assert!(!prompt.contains("c: three"));
}

async fn first_error(input: SummaryInput) -> Option<String> {
    let summarizer = Summarizer::with_stream(Some(config()), None, Arc::new(FakeLlmStream));
    match summarizer.summarize(input).next().await.expect("an event") {
        Ok(_) => None,
        Err(error) => Some(error.to_string()),
    }
}

#[tokio::test]
async fn only_an_empty_article_and_no_comments_is_an_error() {
    let with_both = SummaryInput {
        article: Some("body".to_string()),
        ..input()
    };
    let comments_only = SummaryInput {
        article: None,
        ..input()
    };
    let article_only = SummaryInput {
        comments: vec![],
        article: Some("body".to_string()),
        ..input()
    };
    let blank_article_only = SummaryInput {
        comments: vec![],
        article: Some("   \n".to_string()),
        ..input()
    };
    let neither = SummaryInput {
        comments: vec![],
        article: None,
        ..input()
    };

    assert_eq!(first_error(with_both).await, None);
    assert_eq!(first_error(comments_only).await, None);
    assert_eq!(first_error(article_only).await, None);
    assert_eq!(
        first_error(blank_article_only).await.as_deref(),
        Some("No comments to summarize")
    );
    assert_eq!(
        first_error(neither).await.as_deref(),
        Some("No comments to summarize")
    );
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
