//! Packaging what hntui has already shown for one Story so another agent can
//! continue from there.
//!
//! A Handoff never fetches. It renders the Summary, Article and Comments that
//! are already loaded into one Markdown document, publishes it as a Paste, and
//! puts a single instruction line on the clipboard — short enough to paste into
//! any agent, and self-contained because the raw URL carries its own token.

use crate::api::Story;
use anyhow::{Context, Result};
use futures::future::BoxFuture;
use serde::Deserialize;

/// The paste service. Hard-coded on purpose: it is the user's own service, and
/// a config knob for it would be a second thing to get wrong before `H` works.
const PASTE_API: &str = "https://paste.dzzu.net/api/pastes";

/// A week. Long enough to come back to a thread the next weekend, short enough
/// that a discussion the user read once does not sit on the service forever.
const PASTE_TTL_SECONDS: u64 = 7 * 24 * 60 * 60;

pub(crate) struct PasteRequest {
    pub content: String,
    pub filename: String,
}

/// A published Handoff, addressed by the raw URL an agent can fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Paste {
    pub raw_url: String,
}

pub(crate) trait PasteService: Send + Sync {
    fn create(&self, request: PasteRequest) -> BoxFuture<'static, Result<Paste>>;
}

#[derive(Debug, Clone)]
pub(crate) struct JakePasteService {
    http: reqwest::Client,
}

impl JakePasteService {
    pub(crate) fn new(http: reqwest::Client) -> Self {
        Self { http }
    }
}

#[derive(Deserialize)]
struct PasteResponse {
    #[serde(rename = "rawUrl")]
    raw_url: Option<String>,
}

impl PasteService for JakePasteService {
    fn create(&self, request: PasteRequest) -> BoxFuture<'static, Result<Paste>> {
        let http = self.http.clone();
        Box::pin(async move {
            let response = http
                .post(PASTE_API)
                .timeout(std::time::Duration::from_secs(30))
                .json(&serde_json::json!({
                    "content": request.content,
                    "format": "markdown",
                    "private": true,
                    "ttlSeconds": PASTE_TTL_SECONDS,
                    "filename": request.filename,
                }))
                .send()
                .await
                .with_context(|| format!("POST {PASTE_API}"))?;

            let status = response.status();
            let body = response.text().await.context("read paste response")?;
            anyhow::ensure!(
                status.as_u16() == 201,
                "paste service returned {status}: {}",
                body.chars().take(200).collect::<String>()
            );
            let parsed: PasteResponse =
                serde_json::from_str(&body).context("decode paste response")?;
            let raw_url = parsed.raw_url.context("paste response carried no rawUrl")?;
            Ok(Paste { raw_url })
        })
    }
}

/// What one Handoff carries. Every section is optional because a Handoff
/// carries only what is loaded — an absent section is not a failure.
#[derive(Debug, Default)]
pub(crate) struct HandoffDocument {
    pub summary: Option<HandoffSummary>,
    pub article: Option<String>,
    pub comments: Option<HandoffComments>,
}

/// The discussion as it goes into the document, with the count that describes
/// it. They travel together so the front matter cannot claim a number the
/// Comments section does not back up.
#[derive(Debug)]
pub(crate) struct HandoffComments {
    pub text: String,
    pub count: usize,
}

#[derive(Debug)]
pub(crate) struct HandoffSummary {
    pub text: String,
    pub model: String,
}

impl HandoffDocument {
    /// True when there is nothing loaded worth handing off.
    pub(crate) fn is_empty(&self) -> bool {
        self.summary.is_none() && self.article.is_none() && self.comments.is_none()
    }

    /// The Markdown an agent receives: the front matter `c` copies, then one
    /// section per kind of material that is loaded.
    pub(crate) fn render(&self, story: &Story) -> String {
        let mut out = String::from("---\n");
        out.push_str(&crate::ui::overlay::front_matter_title(&story.title));
        if let Some(url) = &story.url {
            out.push_str(&format!("source: {url}\n"));
        }
        out.push_str(&format!("hn: {}\n", crate::ui::overlay::hn_url(story.id)));
        out.push_str(&format!("score: {}\n", story.score));
        out.push_str(&format!("author: {}\n", story.by));
        // What this document carries, not what the thread holds — an agent that
        // reads `comments: 312` and finds 87 of them reasons about a discussion
        // it was never given. The thread's own total is named separately.
        let carried = self.comments.as_ref().map_or(0, |comments| comments.count);
        out.push_str(&format!("comments: {carried}\n"));
        if story.comment_count as usize != carried {
            out.push_str(&format!("comments_total: {}\n", story.comment_count));
        }
        if let Some(summary) = &self.summary {
            out.push_str(&format!("model: {}\n", summary.model));
        }
        out.push_str(&crate::ui::overlay::front_matter_date(story.time));
        out.push_str("---\n");

        if let Some(summary) = &self.summary {
            out.push_str("\n## Summary\n\n");
            out.push_str(summary.text.trim_end());
            out.push('\n');
        }
        if let Some(article) = &self.article {
            out.push_str("\n## Article\n\n");
            out.push_str(article.trim_end());
            out.push('\n');
        }
        if let Some(comments) = &self.comments {
            out.push_str("\n## Comments\n\n");
            out.push_str(comments.text.trim_end());
            out.push('\n');
        }
        out
    }
}

pub(crate) fn paste_filename(story_id: u64) -> String {
    format!("hn-{story_id}.md")
}

/// The one line that lands on the clipboard. It ends mid-sentence on purpose:
/// the user pastes it into an agent and types the question straight after.
pub(crate) fn instruction_line(raw_url: &str) -> String {
    format!(
        "Read the HN thread handoff at {raw_url} (story, article, comments, AI summary), then help me with: "
    )
}

/// How the footer reports a Handoff. It survives the overlay it was started
/// from: the Paste is already made, and the clipboard is its only side effect.
#[derive(Debug, Clone)]
pub(crate) enum HandoffStatus {
    InFlight,
    Done {
        raw_url: String,
        /// Set when the Paste landed but the clipboard would not take it. The
        /// URL is still the deliverable, so it is reported either way.
        clipboard_error: Option<String>,
    },
    Failed(String),
}

impl HandoffStatus {
    /// In-flight status stays put while the user scrolls; a settled one is
    /// cleared by the next thing they do, like a config reload line.
    pub(crate) fn is_settled(&self) -> bool {
        !matches!(self, HandoffStatus::InFlight)
    }
}

#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct RecordingPasteService {
    requests: std::sync::Mutex<Vec<(String, String)>>,
    fails_with: Option<String>,
}

#[cfg(test)]
impl RecordingPasteService {
    pub(crate) fn failing(message: &str) -> Self {
        Self {
            requests: std::sync::Mutex::new(Vec::new()),
            fails_with: Some(message.to_string()),
        }
    }

    pub(crate) fn last_content(&self) -> Option<String> {
        self.requests().last().map(|(_, content)| content.clone())
    }

    /// The (filename, content) pairs this service was asked to publish.
    pub(crate) fn requests(&self) -> Vec<(String, String)> {
        self.requests
            .lock()
            .expect("recording paste service lock poisoned")
            .clone()
    }
}

#[cfg(test)]
impl PasteService for RecordingPasteService {
    fn create(&self, request: PasteRequest) -> BoxFuture<'static, Result<Paste>> {
        self.requests
            .lock()
            .expect("recording paste service lock poisoned")
            .push((request.filename, request.content));
        let failure = self.fails_with.clone();
        Box::pin(async move {
            match failure {
                Some(message) => anyhow::bail!(message),
                None => Ok(Paste {
                    raw_url: "https://paste.example/raw/abc123?token=t".to_string(),
                }),
            }
        })
    }
}

#[cfg(test)]
mod tests;
