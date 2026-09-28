//! Headless subcommands for agents and scripts. Each prints one JSON document
//! to stdout; failures follow the contract in `error`.

mod error;
#[cfg(test)]
mod tests;

pub use error::{fail, CliError, Result};

use crate::api::types::Comment;
use crate::api::{CommentNode, FeedKind, Story, StorySource};
use crate::article::{self_post_article, Article, ArticleFetcher};
use crate::config::{default_max_article_chars, Config};
use crate::summarizer::{build_prompt, usable_article};
use crate::text::{hn_html_to_article_markdown, hn_html_to_plain};
use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use clap::Subcommand;
use futures::future::BoxFuture;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::time::{SystemTime, UNIX_EPOCH};

pub const AFTER_HELP: &str = "\
Subcommands print one JSON document to stdout. On failure stdout is empty and stderr holds
{\"error\":{\"kind\",\"message\",\"retryable\"}}. Exit codes: 1 service/not_found/local, 2 usage,
4 network. Run bare `hntui` in a terminal for the TUI.";

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// A Feed's Stories. Out: {feed, stories:[{id, title, url, hn_url, by, score,
    /// comments, time, type}]}
    Stories {
        /// Feed to list
        #[arg(long, value_enum, default_value = "top")]
        feed: FeedKind,
        /// Max Stories, taken from the top of the Feed
        #[arg(short = 'n', long, default_value = "30")]
        limit: NonZeroUsize,
        /// Keep only Stories posted at or after this: unix seconds, RFC 3339,
        /// YYYY-MM-DD (UTC), or 30m/24h/7d ago. Filters the --limit Stories.
        #[arg(long, value_parser = parse_since)]
        since: Option<i64>,
    },
    /// One Story with its Article and Comments. Out: {story:{…Story}, body,
    /// article:{title, url, content}|null, article_error, comments:[{id, by, time,
    /// text, replies:[…]}], comments_included, text}. `text` is the Summarizer's prompt.
    Thread {
        /// Story id (the number in item?id=<id>)
        id: u64,
        /// Max Comments, in reading order (0 = all)
        #[arg(short = 'n', long, default_value_t = crate::config::default_max_comments())]
        max_comments: usize,
        /// Fetch the linked Article via localwebrs (default)
        #[arg(long, overrides_with = "no_article")]
        article: bool,
        /// Skip the Article: no localwebrs subprocess
        #[arg(long)]
        no_article: bool,
    },
}

pub async fn run(
    command: Command,
    source: &dyn StorySource,
    fetcher: &ArticleFetcher,
    config: &Config,
) -> Result<Value> {
    match command {
        Command::Stories { feed, limit, since } => stories(source, feed, limit.get(), since).await,
        Command::Thread {
            id,
            max_comments,
            no_article,
            ..
        } => {
            let max_article_chars = config
                .summarize()
                .map_or_else(default_max_article_chars, |s| s.max_article_chars);
            let budget = if max_comments == 0 {
                usize::MAX
            } else {
                max_comments
            };
            let fetcher = (!no_article).then_some(fetcher);
            thread(source, fetcher, id, budget, max_article_chars).await
        }
    }
}

pub(crate) async fn stories(
    source: &dyn StorySource,
    feed: FeedKind,
    limit: usize,
    since: Option<i64>,
) -> Result<Value> {
    let failed = |e: anyhow::Error| CliError::from_source(&e);
    let (ids, mut stories) = source.initial_stories(feed, limit).await.map_err(failed)?;
    while stories.len() < limit {
        let more = source
            .more_stories(feed, ids.clone(), stories.len(), limit - stories.len())
            .await
            .map_err(failed)?;
        if more.is_empty() {
            break;
        }
        stories.extend(more);
    }
    // Pages can overlap when the ranking shifts between requests.
    let mut seen = HashSet::new();
    let stories: Vec<Value> = stories
        .iter()
        .filter(|story| seen.insert(story.id))
        .take(limit)
        .filter(|story| since.is_none_or(|since| story.time >= since))
        .map(story_json)
        .collect();
    Ok(json!({ "feed": feed.as_str(), "stories": stories }))
}

pub(crate) async fn thread(
    source: &dyn StorySource,
    fetcher: Option<&ArticleFetcher>,
    id: u64,
    max_comments: usize,
    max_article_chars: usize,
) -> Result<Value> {
    let failed = |e: anyhow::Error| CliError::from_source(&e);
    let (mut story, thread) = source.story_thread(id).await.map_err(failed)?;
    story.absorb_text(thread.text);
    let mut budget = max_comments;
    let roots = load_comments(source, thread.comments, &mut budget)
        .await
        .map_err(failed)?;
    let mut flat = Vec::new();
    let comments = comments_json(&roots, &mut flat);

    let (article, article_error) = match fetcher {
        Some(fetcher) => fetch_article(&story, fetcher).await,
        None => (None, None),
    };
    let content = article.as_ref().map(|article| article.content.as_str());
    let text = build_prompt(
        &story,
        &flat,
        usable_article(content),
        flat.len(),
        max_article_chars,
    );
    Ok(json!({
        "story": story_json(&story),
        "body": story.text.as_deref().map(hn_html_to_article_markdown),
        "article": article.map(|article| json!({
            "title": article.title,
            "url": article.effective_url.or(story.url.clone()),
            "content": article.content,
        })),
        "article_error": article_error,
        "comments": comments,
        "comments_included": flat.len(),
        "text": text,
    }))
}

/// The Article as `v` would show it: the linked page through localwebrs, or a
/// self-post's own body. A failed fetch is reported, never dropped.
async fn fetch_article(
    story: &Story,
    fetcher: &ArticleFetcher,
) -> (Option<Article>, Option<String>) {
    match &story.url {
        Some(url) => match fetcher.fetch(url.clone()).await {
            Ok(article) => (Some(article), None),
            Err(error) => (None, Some(error.to_string())),
        },
        None => (self_post_article(story), None),
    }
}

/// Walk the discussion in reading order, fetching replies the Source has not
/// loaded yet (Firebase stops a few levels deep), until `budget` Comments are
/// in hand. Everything past the budget is dropped.
fn load_comments<'a>(
    source: &'a dyn StorySource,
    nodes: Vec<CommentNode>,
    budget: &'a mut usize,
) -> BoxFuture<'a, anyhow::Result<Vec<CommentNode>>> {
    Box::pin(async move {
        let mut kept = Vec::new();
        for mut node in nodes {
            if *budget == 0 {
                break;
            }
            *budget -= 1;
            let comment = &node.comment;
            if !comment.children_loaded && !comment.kids.is_empty() {
                node.children = source
                    .comment_children(comment.kids.clone(), comment.depth + 1)
                    .await?;
            }
            node.children =
                load_comments(source, std::mem::take(&mut node.children), budget).await?;
            kept.push(node);
        }
        Ok(kept)
    })
}

/// The comment tree as JSON, collecting the same Comments in reading order
/// for the prompt.
fn comments_json(nodes: &[CommentNode], flat: &mut Vec<Comment>) -> Vec<Value> {
    nodes
        .iter()
        .map(|node| {
            let comment = &node.comment;
            flat.push(comment.clone());
            json!({
                "id": comment.id,
                "by": comment.by,
                "time": comment.time.map(iso_time),
                "text": hn_html_to_plain(&comment.text),
                "replies": comments_json(&node.children, flat),
            })
        })
        .collect()
}

fn story_json(story: &Story) -> Value {
    json!({
        "id": story.id,
        "title": story.title,
        "url": story.url,
        "hn_url": format!("https://news.ycombinator.com/item?id={}", story.id),
        "by": (!story.by.is_empty()).then_some(&story.by),
        "score": story.score,
        "comments": story.comment_count,
        "time": iso_time(story.time),
        "type": story.kind,
    })
}

/// RFC 3339 in UTC; unix seconds when out of chrono's range.
fn iso_time(secs: i64) -> Value {
    match DateTime::<Utc>::from_timestamp(secs, 0) {
        Some(time) => json!(time.to_rfc3339_opts(SecondsFormat::Secs, true)),
        None => json!(secs),
    }
}

fn now_secs() -> i64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before 1970");
    i64::try_from(elapsed.as_secs()).expect("system clock in i64 range")
}

fn parse_since(text: &str) -> std::result::Result<i64, String> {
    parse_since_at(text, now_secs())
}

/// Unix seconds, RFC 3339, `YYYY-MM-DD` (UTC midnight), or `<n>m|h|d` ago.
fn parse_since_at(text: &str, now: i64) -> std::result::Result<i64, String> {
    if let Ok(secs) = text.parse::<i64>() {
        return Ok(secs);
    }
    if let Ok(time) = DateTime::parse_from_rfc3339(text) {
        return Ok(time.timestamp());
    }
    if let Ok(day) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        let midnight = day.and_hms_opt(0, 0, 0).expect("midnight exists");
        return Ok(midnight.and_utc().timestamp());
    }
    let ago = [('m', 60), ('h', 3_600), ('d', 86_400)]
        .into_iter()
        .find_map(|(suffix, unit)| Some((text.strip_suffix(suffix)?.parse::<i64>().ok()?, unit)));
    match ago {
        Some((count, unit)) if count >= 0 => Ok(now - count * unit),
        _ => Err(format!(
            "expected unix seconds, RFC 3339, YYYY-MM-DD, or 30m/24h/7d; got {text:?}"
        )),
    }
}
