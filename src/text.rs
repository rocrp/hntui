use html_escape::{decode_html_entities, encode_text};
use regex::{Captures, Regex};
use std::sync::LazyLock;

static HN_ANCHOR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<a\b[^>]*\bhref\s*=\s*(?:"([^"]*)"|'([^']*)')[^>]*>(.*?)</a\s*>"#)
        .expect("HN anchor regex must compile")
});

/// Preserve the one piece of HN's sanitized HTML that Article navigation
/// needs, then reuse the plain-text normalization used elsewhere.
pub(crate) fn hn_html_to_article_markdown(html: &str) -> String {
    let markdown = HN_ANCHOR_RE.replace_all(html, |captures: &Captures<'_>| {
        let encoded_href = captures
            .get(1)
            .or_else(|| captures.get(2))
            .expect("anchor regex must capture href")
            .as_str();
        let label_html = captures
            .get(3)
            .expect("anchor regex must capture label")
            .as_str();

        let label = escape_markdown_label(&hn_html_to_plain(label_html));
        let label = encode_text(&label);
        let href = decode_html_entities(encoded_href);
        let href = escape_markdown_destination(&href);
        let href = encode_text(&href);
        format!("[{label}]({href})")
    });
    hn_html_to_plain(&markdown)
}

fn escape_markdown_label(label: &str) -> String {
    let mut escaped = String::with_capacity(label.len());
    for character in label.chars() {
        if character.is_ascii_punctuation() {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn escape_markdown_destination(destination: &str) -> String {
    let mut escaped = String::with_capacity(destination.len());
    for character in destination.chars() {
        if matches!(character, '\\' | '(' | ')') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

pub(crate) fn hn_html_to_plain(html: &str) -> String {
    let html = html
        .replace("<p>", "\n\n")
        .replace("</p>", "\n\n")
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n");

    let mut stripped = String::with_capacity(html.len());
    let mut in_tag = false;
    for character in html.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => stripped.push(character),
            _ => {}
        }
    }

    let decoded = decode_html_entities(&stripped).into_owned();
    let mut result = Vec::new();
    let mut previous_empty = false;
    for line in decoded.lines() {
        let trimmed = collapse_spaces(line.trim());
        if trimmed.is_empty() {
            if !previous_empty && !result.is_empty() {
                result.push(String::new());
                previous_empty = true;
            }
        } else {
            result.push(trimmed);
            previous_empty = false;
        }
    }
    while result.last().is_some_and(String::is_empty) {
        result.pop();
    }
    result.join("\n")
}

fn collapse_spaces(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut previous_space = false;
    for character in value.chars() {
        if character.is_whitespace() {
            if !previous_space {
                output.push(' ');
            }
            previous_space = true;
        } else {
            previous_space = false;
            output.push(character);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_paragraphs_breaks_and_entities() {
        let html = "<p>Hello&nbsp;world</p><p>line<br>next &amp; more</p>";
        assert_eq!(hn_html_to_plain(html), "Hello world\n\nline\nnext & more");
    }

    #[test]
    fn keeps_link_text_and_strips_tags() {
        let html = r#"Read <a href="https://example.com">this</a> &gt; that"#;
        assert_eq!(hn_html_to_plain(html), "Read this > that");
    }

    #[test]
    fn article_markdown_preserves_hn_anchor_targets() {
        let html = concat!(
            "<p>Read <a href=\"https:&#x2F;&#x2F;example.com&#x2F;docs?a=1&amp;b=2\" ",
            "rel=\"nofollow\"><code>the docs</code></a>.</p>",
            "<p>Then continue.</p>"
        );

        assert_eq!(
            hn_html_to_article_markdown(html),
            "Read [the docs](https://example.com/docs?a=1&b=2).\n\nThen continue."
        );
    }

    #[test]
    fn collapses_whitespace_and_trailing_blank_lines() {
        let html = "<p>  alpha   beta  </p><p></p><p> gamma </p><br><br>";
        assert_eq!(hn_html_to_plain(html), "alpha beta\n\ngamma");
    }
}
