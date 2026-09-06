//! How a Story's Article and Comments are rendered as text for a model.
//!
//! The Summarizer's prompt and a Handoff document must agree byte for byte on
//! this: a Handoff claims to carry what the Summarizer saw, and that claim is
//! only true while both read the material through here.

use crate::api::types::Comment;
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
