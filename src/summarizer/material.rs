//! How a Story's Article and Comments are rendered as text for a model.
//!
//! The Summarizer's prompt and a Handoff document must agree byte for byte on
//! this: a Handoff claims to carry what the Summarizer saw, and that claim is
//! only true while both read the material through here. `hntui thread`'s
//! `text` is this same prompt, so an agent reads what the Summarizer would.

use crate::api::types::{Comment, Story};
use crate::text::hn_html_to_plain;

/// The discussion as indented `author: text` paragraphs, capped at
/// `max_comments`. Ends with a blank line, as each paragraph does.
pub(crate) fn comments_as_thread(comments: &[Comment], max_comments: usize) -> String {
    let mut rendered = String::new();
    for comment in comments.iter().take(max_comments) {
        let author = comment.by.as_deref().unwrap_or("[anon]");
        let indent = "  ".repeat(comment.depth);
        let text = hn_html_to_plain(&comment.text);
        rendered.push_str(&format!("{indent}{author}: {text}\n\n"));
    }
    rendered
}

/// Head-truncate on a char boundary; the lead of an article carries the thesis.
pub(crate) fn truncated_article(article: &str, max_chars: usize) -> String {
    let mut truncated: String = article.chars().take(max_chars).collect();
    if truncated.chars().count() < article.chars().count() {
        truncated.push_str("\n\n…[truncated]");
    }
    truncated
}

/// The Summarizer's user prompt: title, the Article when there is one, then
/// the discussion.
pub(crate) fn build_prompt(
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
