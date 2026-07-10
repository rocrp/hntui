use html_escape::decode_html_entities;

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
    fn collapses_whitespace_and_trailing_blank_lines() {
        let html = "<p>  alpha   beta  </p><p></p><p> gamma </p><br><br>";
        assert_eq!(hn_html_to_plain(html), "alpha beta\n\ngamma");
    }
}
