use super::*;

fn story() -> Story {
    Story {
        id: 42,
        title: "A story".to_string(),
        url: Some("https://example.com".to_string()),
        text: None,
        score: 99,
        by: "alice".to_string(),
        time: 1_700_000_000,
        comment_count: 7,
        kids: vec![],
    }
}

#[test]
fn a_summary_only_document_pairs_front_matter_with_one_section() {
    let document = HandoffDocument {
        summary: Some(HandoffSummary {
            text: "It is about widgets.".to_string(),
            model: "fake/model".to_string(),
        }),
        ..Default::default()
    };

    assert_eq!(
        document.render(&story()),
        "---\n\
         title: \"A story\"\n\
         source: https://example.com\n\
         hn: https://news.ycombinator.com/item?id=42\n\
         score: 99\n\
         author: alice\n\
         comments: 7\n\
         model: fake/model\n\
         date: 2023-11-14\n\
         ---\n\
         \n\
         ## Summary\n\
         \n\
         It is about widgets.\n"
    );
}

#[test]
fn without_a_summary_the_front_matter_names_no_model() {
    let document = HandoffDocument {
        article: Some("The article text.".to_string()),
        ..Default::default()
    };

    let rendered = document.render(&story());

    assert!(!rendered.contains("model:"), "{rendered}");
    assert!(
        rendered.contains("\n## Article\n\nThe article text.\n"),
        "{rendered}"
    );
}

#[test]
fn sections_appear_in_summary_article_comments_order() {
    let document = HandoffDocument {
        summary: Some(HandoffSummary {
            text: "S".to_string(),
            model: "m".to_string(),
        }),
        article: Some("A".to_string()),
        comments: Some("C".to_string()),
    };

    let rendered = document.render(&story());
    let summary = rendered.find("## Summary").expect("summary section");
    let article = rendered.find("## Article").expect("article section");
    let comments = rendered.find("## Comments").expect("comments section");

    assert!(summary < article && article < comments, "{rendered}");
}

#[test]
fn material_that_is_not_loaded_leaves_no_empty_heading() {
    let document = HandoffDocument {
        comments: Some("bob: hi".to_string()),
        ..Default::default()
    };

    let rendered = document.render(&story());

    assert!(!rendered.contains("## Summary"), "{rendered}");
    assert!(!rendered.contains("## Article"), "{rendered}");
    assert!(rendered.ends_with("## Comments\n\nbob: hi\n"), "{rendered}");
}

#[test]
fn a_story_without_a_link_carries_no_source_line() {
    let mut story = story();
    story.url = None;
    let document = HandoffDocument {
        comments: Some("bob: hi".to_string()),
        ..Default::default()
    };

    assert!(!document.render(&story).contains("source:"));
}

#[test]
fn an_empty_document_knows_it_has_nothing_to_hand_off() {
    assert!(HandoffDocument::default().is_empty());
}

#[test]
fn the_instruction_line_ends_open_for_the_users_question() {
    let line = instruction_line("https://paste.example/raw/x?token=t");

    assert_eq!(
        line,
        "Read the HN thread handoff at https://paste.example/raw/x?token=t \
         (story, article, comments, AI summary), then help me with: "
    );
}

#[test]
fn the_paste_is_named_after_the_story() {
    assert_eq!(paste_filename(42), "hn-42.md");
}
