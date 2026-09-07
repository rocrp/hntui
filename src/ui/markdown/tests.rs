use super::*;

/// Wide enough that no test table is squeezed by the fitting pass.
const TEST_WIDTH: u16 = 80;

fn line_texts(input: &str) -> Vec<String> {
    line_texts_at(input, TEST_WIDTH)
}

fn line_texts_at(input: &str, width: u16) -> Vec<String> {
    render_markdown(input, width)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.into_owned())
                .collect::<String>()
        })
        .collect()
}

#[test]
fn renders_heading_and_paragraph_spacing() {
    let lines = line_texts("# Title\n\nBody **strong** and *em*");

    assert_eq!(lines, vec!["Title", "", "Body strong and em"]);
}

#[test]
fn renders_nested_unordered_and_ordered_lists() {
    let lines = line_texts("- parent\n  - child\n\n3. third\n4. fourth");

    assert_eq!(
        lines,
        vec!["- parent", "  - child", "", "3. third", "4. fourth"]
    );
}

#[test]
fn renders_block_quotes_and_code_blocks() {
    let lines = line_texts("> quoted\n\n```\nlet x = 1;\n```");

    assert_eq!(lines, vec!["> quoted", "", "  let x = 1;"]);
}

fn styled_texts(input: &str) -> Vec<(String, bool, bool)> {
    render_markdown(input, TEST_WIDTH)
        .into_iter()
        .flat_map(|line| line.spans)
        .map(|span| {
            (
                span.content.into_owned(),
                span.style.add_modifier.contains(Modifier::BOLD),
                span.style.add_modifier.contains(Modifier::ITALIC),
            )
        })
        .collect()
}

fn bold_runs(input: &str) -> Vec<String> {
    styled_texts(input)
        .into_iter()
        .filter(|(_, bold, _)| *bold)
        .map(|(text, _, _)| text)
        .collect()
}

fn italic_runs(input: &str) -> Vec<String> {
    styled_texts(input)
        .into_iter()
        .filter(|(_, _, italic)| *italic)
        .map(|(text, _, _)| text)
        .collect()
}

#[test]
fn emphasis_closes_against_chinese_punctuation() {
    assert_eq!(bold_runs("**要点：**内容"), vec!["要点："]);
    assert_eq!(bold_runs("**粗体、**然后"), vec!["粗体、"]);
    assert_eq!(bold_runs("这是**「引用」**的例子"), vec!["「引用」"]);
    assert_eq!(bold_runs("**结论——**很好"), vec!["结论——"]);
}

#[test]
fn chinese_emphasis_fix_leaves_the_surrounding_text_intact() {
    assert_eq!(line_texts("**要点：**内容在这里"), vec!["要点：内容在这里"]);
}

#[test]
fn emphasis_that_already_worked_still_works() {
    assert_eq!(bold_runs("**要点**：内容"), vec!["要点"]);
    assert_eq!(bold_runs("**粗体**。然后"), vec!["粗体"]);
    assert_eq!(bold_runs("**要点：** 内容"), vec!["要点："]);
    assert_eq!(italic_runs("*斜体*，然后"), vec!["斜体"]);
}

#[test]
fn ascii_punctuation_keeps_strict_commonmark_flanking() {
    assert_eq!(line_texts("**Note:**text"), vec!["**Note:**text"]);
    assert!(bold_runs("**Note:**text").is_empty());
    assert_eq!(line_texts("snake_case_name"), vec!["snake_case_name"]);
    assert!(italic_runs("snake_case_name").is_empty());
}

#[test]
fn chinese_punctuation_survives_code_and_link_destinations() {
    assert_eq!(line_texts("`**要点：**`"), vec!["`**要点：**`"]);
    assert_eq!(line_texts("```\n要点：内容\n```"), vec!["  要点：内容"]);

    let document =
        render_markdown_document("[链接](https://example.com/a、b)", None, None, TEST_WIDTH);

    assert_eq!(document.links, vec!["https://example.com/a%E3%80%81b"]);
}

#[test]
fn documents_already_using_the_placeholder_range_are_left_strict() {
    let input = "\u{e0a0} **要点：**内容";

    let text = line_texts(input).join("");

    assert!(text.contains('\u{e0a0}'));
    assert!(text.contains("**"));
}

const TABLE: &str = "\
| Feature | Status | Notes |
|---|:---:|---:|
| Bold | done | ships now |
| Tables | wip | later |";

#[test]
fn renders_a_table_as_aligned_columns_with_a_header_rule() {
    let lines = line_texts(TABLE);

    assert_eq!(
        lines,
        vec![
            "Feature  Status      Notes",
            "───────  ──────  ─────────",
            "Bold      done   ships now",
            "Tables    wip        later",
        ]
    );
}

#[test]
fn table_header_is_bold_and_the_body_is_not() {
    let styled = styled_texts(TABLE);
    let bold: Vec<String> = styled
        .iter()
        .filter(|(text, bold, _)| *bold && !text.trim().is_empty())
        .map(|(text, _, _)| text.trim().to_string())
        .collect();

    assert_eq!(bold, vec!["Feature", "Status", "Notes"]);
    assert!(!styled
        .iter()
        .any(|(text, bold, _)| *bold && text.contains("Bold")));
}

#[test]
fn table_columns_honour_the_alignment_markers() {
    let lines = line_texts(TABLE);

    // Left column flush left, centre column padded on both sides, right
    // column flush right.
    assert!(lines[2].starts_with("Bold"));
    assert!(lines[2].contains("  done  "));
    assert!(lines[2].ends_with("ships now"));
}

#[test]
fn a_table_wider_than_the_viewport_is_squeezed_to_fit() {
    let width = 24;
    let lines = render_markdown(TABLE, width);

    for line in &lines {
        assert!(
            line.width() <= usize::from(width),
            "line {:?} is {} wide",
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            line.width()
        );
    }
    let texts = line_texts_at(TABLE, width);
    assert!(
        texts.iter().any(|text| text.contains('…')),
        "expected an ellipsis in {texts:?}"
    );
}

#[test]
fn cjk_table_cells_align_by_display_width() {
    let lines = line_texts("| 语言 | Notes |\n|---|---|\n| 中文 | ok |\n| a | b |");

    let widths: Vec<usize> = lines
        .iter()
        .map(|line| UnicodeWidthStr::width(line.as_str()))
        .collect();
    assert_eq!(widths[0], widths[2]);
    assert_eq!(widths[2], widths[3]);
}

#[test]
fn a_table_still_renders_when_the_viewport_width_is_unknown() {
    let lines = render_markdown(TABLE, 0);

    assert_eq!(lines.len(), 4);
}

#[test]
fn renders_links_as_underlined_text_without_url_suffix() {
    let lines = render_markdown("[site](https://example.com)", TEST_WIDTH);

    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].spans[0].content, "site");
    assert!(lines[0].spans[0]
        .style
        .add_modifier
        .contains(Modifier::UNDERLINED));
}

/// Every prefix of a document is what the streaming summary actually renders,
/// so each one has to survive at every width the overlay can be.
#[test]
fn every_prefix_of_a_document_renders_without_panicking_or_leaking() {
    let document = "# 标题\n\n**要点：**内容——很好\n\n\
                    - **性能：**提升\n\n\
                    | 观点 | 支持 | 说明 |\n|---|:---:|---:|\n\
                    | 所有权 | 多数 | 减少竞争 |\n\n\
                    > 引用「话」\n\n\
                    `代码：x` and [链接](https://example.com/a、b)\n\n\
                    ```\n块：内容\n```\n";

    for end in 0..=document.len() {
        if !document.is_char_boundary(end) {
            continue;
        }
        let prefix = &document[..end];
        for width in [0u16, 1, 2, 3, 7, 20, 80] {
            let lines = render_markdown(prefix, width);
            for line in &lines {
                let text: String = line
                    .spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect();
                assert!(
                    !text
                        .chars()
                        .any(|character| ('\u{e000}'..='\u{f8ff}').contains(&character)),
                    "placeholder leaked at width {width} for prefix {prefix:?}: {text:?}"
                );
            }
        }
    }
}

/// The escaping in [`super::cjk`] rewrites the document before CommonMark
/// sees it, so it can take emphasis away as easily as it grants it. Every
/// shape stock CommonMark already renders has to survive: a sweep beats
/// hand-picked cases here, because the losing shapes (`**50%**，`) look
/// nothing like the winning ones (`**要点：**内容`).
#[test]
fn escaping_never_costs_emphasis_that_commonmark_already_rendered() {
    let openers = ["", "文字", "他说", "详见 ", "增长 "];
    let contents = ["核心", "50%", "MIT (开源)", "已废弃.", "重点", "要点："];
    let tails = ["", "：", "、", "。", "——", "」", "”", "%", ")", "."];
    let afters = ["", "内容", "，主要", "。", "」", "然后", " 内容", "**B**"];
    let delimiters = ["**", "*", "~~", "_", "__"];

    let mut fixes = 0;
    let mut regressions = Vec::new();
    for delimiter in delimiters {
        for opener in openers {
            for content in contents {
                for tail in tails {
                    for after in afters {
                        let input = format!("{opener}{delimiter}{content}{tail}{delimiter}{after}");
                        let strict = emphasised_text(&input, false);
                        let relaxed = emphasised_text(&input, true);
                        // Extra runs are the point of the relaxation; a run
                        // that stock CommonMark rendered going missing is not.
                        if relaxed.len() > strict.len() {
                            fixes += 1;
                        }
                        if let Some(lost) = strict.iter().find(|run| !relaxed.contains(run)) {
                            regressions.push(format!("{input:?} lost {lost:?}"));
                        }
                    }
                }
            }
        }
    }

    assert!(regressions.is_empty(), "lost emphasis on {regressions:#?}");
    assert!(fixes > 0, "the relaxation stopped fixing anything");
}

/// The emphasised runs of `input`, as plain text. `relaxed` picks the
/// renderer's own path; otherwise the document is parsed exactly as
/// CommonMark specifies, which is the baseline the sweep compares against.
fn emphasised_text(input: &str, relaxed: bool) -> Vec<String> {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

    let (source, _) = if relaxed {
        super::cjk::escape_cjk_punctuation(input)
    } else {
        (std::borrow::Cow::Borrowed(input), false)
    };
    let mut depth = 0usize;
    let mut runs = Vec::new();
    let mut current = String::new();
    for event in Parser::new_ext(&source, Options::ENABLE_STRIKETHROUGH) {
        match event {
            Event::Start(Tag::Strong | Tag::Emphasis | Tag::Strikethrough) => depth += 1,
            Event::End(TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough) => {
                depth -= 1;
                if depth == 0 && !current.is_empty() {
                    runs.push(super::cjk::restore_cjk_punctuation(&current, relaxed));
                    current.clear();
                }
            }
            Event::Text(text) | Event::Code(text) if depth > 0 => current.push_str(&text),
            _ => {}
        }
    }
    runs
}

/// The shapes a wholesale escape breaks. Each one renders under stock
/// CommonMark because the CJK punctuation *beside* the run is what makes the
/// run flanking, so the escape has to leave these alone.
#[test]
fn emphasis_rescued_by_neighbouring_chinese_punctuation_survives() {
    assert_eq!(
        bold_runs("本季度收入增长 **50%**，主要来自海外市场。"),
        vec!["50%"]
    );
    assert_eq!(
        bold_runs("该项目采用 **MIT (开源)**，可自由使用。"),
        vec!["MIT (开源)"]
    );
    assert_eq!(
        bold_runs("详见 **[官方文档](https://example.com)**。"),
        vec!["官方文档"]
    );
    assert_eq!(italic_runs("他说“_重要_”内容"), vec!["重要"]);
    assert_eq!(italic_runs("文字——_重点_——文字"), vec!["重点"]);
    assert_eq!(line_texts("*`x`*，然后"), vec!["`x`，然后"]);
    assert_eq!(line_texts("~~已废弃.~~，然后"), vec!["已废弃.，然后"]);
}

/// A numeric character reference is decoded after the escape pass, so it can
/// conjure a private-use character the restore step would mistake for a
/// placeholder. Such a document keeps strict parsing rather than risk it.
#[test]
fn numeric_character_references_cannot_forge_a_placeholder() {
    assert_eq!(line_texts("你好，&#xE000;世界"), vec!["你好，\u{e000}世界"]);

    let document = render_markdown_document(
        "[x](https://example.com/&#xE005;)（注）",
        None,
        None,
        TEST_WIDTH,
    );

    assert_eq!(document.links, vec!["https://example.com/%EE%80%85"]);
}

#[test]
fn a_table_inside_a_block_quote_keeps_its_marker() {
    let lines = line_texts("> | A | B |\n> |---|---|\n> | 1 | 2 |");

    assert_eq!(lines, vec!["> A  B", "> ─  ─", "> 1  2"]);
}
