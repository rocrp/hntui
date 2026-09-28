use super::*;

fn ask_hn_item() -> HnItem {
    HnItem {
        id: 7,
        kind: Some(HnItemKind::Story),
        by: Some("alice".to_string()),
        time: Some(1),
        title: Some("Ask HN: anything?".to_string()),
        url: None,
        text: Some("<p>the body".to_string()),
        score: Some(1),
        descendants: Some(0),
        kids: None,
        dead: None,
        deleted: None,
    }
}

#[test]
fn self_post_body_survives_the_item_to_story_conversion() {
    let story = Story::try_from(ask_hn_item()).expect("ask hn item is a story");

    assert_eq!(story.text.as_deref(), Some("<p>the body"));
    assert_eq!(story.url, None);
}

#[test]
fn hackerweb_item_carries_the_self_post_body_with_the_discussion() {
    let payload = r#"{"content":"<p>the body","comments":[]}"#;
    let item: WebItem = serde_json::from_str(payload).expect("decode hackerweb item");

    let thread = StoryThread {
        text: item.content.filter(|text| !text.trim().is_empty()),
        comments: Vec::new(),
    };

    assert_eq!(thread.text.as_deref(), Some("<p>the body"));
}

#[test]
fn hackerweb_internal_discussion_url_is_not_an_article_link() {
    let payload = r#"{
        "id": 7,
        "title": "Ask HN: anything?",
        "url": "item?id=7",
        "points": 1,
        "user": "alice",
        "time": 1,
        "comments_count": 0,
        "type": "ask"
    }"#;
    let listed: WebStory = serde_json::from_str(payload).expect("decode hackerweb listing");

    let story = Story::from(listed);

    assert_eq!(story.url, None);
}

#[test]
fn hackerweb_external_article_url_survives_the_listing_conversion() {
    let payload = r#"{
        "id": 7,
        "title": "A linked article",
        "url": "https://example.com/article",
        "points": 1,
        "user": "alice",
        "time": 1,
        "comments_count": 0,
        "type": "link"
    }"#;
    let listed: WebStory = serde_json::from_str(payload).expect("decode hackerweb listing");

    let story = Story::from(listed);

    assert_eq!(story.url.as_deref(), Some("https://example.com/article"));
}

#[test]
fn a_link_submission_has_no_body_in_either_shape() {
    let mut item = ask_hn_item();
    item.text = None;
    item.url = Some("https://example.com".to_string());
    let story = Story::try_from(item).expect("link item is a story");

    let payload = r#"{"comments":[]}"#;
    let web_item: WebItem = serde_json::from_str(payload).expect("decode hackerweb item");

    assert_eq!(story.text, None);
    assert_eq!(web_item.content, None);
}

#[test]
fn absorb_text_fills_a_missing_body_but_never_overwrites_one() {
    let mut listed = Story::try_from(ask_hn_item()).expect("story");
    listed.text = None;
    listed.absorb_text(Some("<p>from the thread".to_string()));
    assert_eq!(listed.text.as_deref(), Some("<p>from the thread"));

    listed.absorb_text(Some("<p>later, ignored".to_string()));
    assert_eq!(listed.text.as_deref(), Some("<p>from the thread"));

    let mut blank = Story::try_from(ask_hn_item()).expect("story");
    blank.text = None;
    blank.absorb_text(Some("   ".to_string()));
    assert_eq!(blank.text, None);
}
