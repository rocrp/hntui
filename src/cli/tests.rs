use super::*;
use crate::api::types::StoryKind;
use crate::api::{InMemorySource, StoryLookupError};
use crate::Cli;
use clap::{CommandFactory, Parser};

fn story(id: u64, time: i64) -> Story {
    Story {
        id,
        title: format!("story {id}"),
        url: Some(format!("https://example.com/{id}")),
        text: None,
        score: 10,
        by: "alice".to_string(),
        time,
        comment_count: 3,
        kids: vec![],
        kind: StoryKind::Story,
    }
}

fn node(id: u64, depth: usize, kids: Vec<u64>, children: Vec<CommentNode>) -> CommentNode {
    CommentNode {
        comment: Comment {
            id,
            by: Some(format!("user{id}")),
            time: Some(0),
            text: format!("<p>comment {id}"),
            children_loaded: kids.is_empty() || !children.is_empty(),
            kids,
            depth,
            collapsed: true,
            children_loading: false,
        },
        children,
    }
}

/// 10 ─┬─ 11 ── 12
///     └─ 13
/// 20
fn discussion() -> Vec<CommentNode> {
    vec![
        node(
            10,
            0,
            vec![11, 13],
            vec![
                node(11, 1, vec![12], vec![node(12, 2, vec![], vec![])]),
                node(13, 1, vec![], vec![]),
            ],
        ),
        node(20, 0, vec![], vec![]),
    ]
}

fn missing_bin() -> ArticleFetcher {
    ArticleFetcher::new("/nonexistent/hntui-test-localwebrs".to_string(), None)
}

#[test]
fn cli_definition_is_valid() {
    Cli::command().debug_assert();
}

#[test]
fn subcommands_parse_and_bare_stays_the_tui() {
    let parse = |args: &[&str]| Cli::try_parse_from(args).map(|cli| cli.command);
    assert!(parse(&["hntui"]).unwrap().is_none());
    assert!(matches!(
        parse(&["hntui", "stories", "--feed", "ask", "-n", "5", "--since", "7d"]).unwrap(),
        Some(Command::Stories { feed: FeedKind::Ask, limit, since: Some(_) }) if limit.get() == 5
    ));
    assert!(matches!(
        parse(&["hntui", "thread", "42", "--no-article"]).unwrap(),
        Some(Command::Thread {
            id: 42,
            max_comments: 200,
            no_article: true,
            ..
        })
    ));
    assert!(matches!(
        parse(&["hntui", "thread", "42", "--no-article", "--article"]).unwrap(),
        Some(Command::Thread {
            no_article: false,
            ..
        })
    ));
    assert!(parse(&["hntui", "stories", "--limit", "0"]).is_err());
    assert!(parse(&["hntui", "stories", "--since", "yesterday"]).is_err());
    assert!(parse(&["hntui", "thread", "abc"]).is_err());
}

#[test]
fn since_accepts_unix_iso_date_and_relative() {
    let now = 1_000_000;
    assert_eq!(parse_since_at("1700000000", now), Ok(1_700_000_000));
    assert_eq!(
        parse_since_at("2024-01-01T00:00:00Z", now),
        Ok(1_704_067_200)
    );
    assert_eq!(
        parse_since_at("2024-01-01T08:00:00+08:00", now),
        Ok(1_704_067_200)
    );
    assert_eq!(parse_since_at("2024-01-01", now), Ok(1_704_067_200));
    assert_eq!(parse_since_at("30m", now), Ok(now - 1_800));
    assert_eq!(parse_since_at("24h", now), Ok(now - 86_400));
    assert_eq!(parse_since_at("7d", now), Ok(now - 604_800));
    assert!(parse_since_at("7w", now).is_err());
    assert!(parse_since_at("-1d", now).is_err());
    assert!(parse_since_at("né", now).is_err());
}

#[tokio::test]
async fn stories_are_limited_then_filtered_by_since() {
    let source = InMemorySource::new(vec![story(1, 100), story(2, 50), story(3, 200)]);

    let all = stories(&source, FeedKind::Top, 2, None).await.unwrap();
    let recent = stories(&source, FeedKind::Top, 3, Some(100)).await.unwrap();

    assert_eq!(all["feed"], "top");
    assert_eq!(all["stories"][0]["id"], 1);
    assert_eq!(all["stories"].as_array().unwrap().len(), 2);
    let ids: Vec<_> = recent["stories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|story| story["id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids, [1, 3]);
    assert_eq!(
        all["stories"][0],
        json!({
            "id": 1,
            "title": "story 1",
            "url": "https://example.com/1",
            "hn_url": "https://news.ycombinator.com/item?id=1",
            "by": "alice",
            "score": 10,
            "comments": 3,
            "time": "1970-01-01T00:01:40Z",
            "type": "story",
        })
    );
}

#[tokio::test]
async fn thread_nests_comments_and_text_is_the_summarizer_prompt() {
    let source = InMemorySource::new(vec![story(1, 0)]).with_comments(1, discussion());

    let out = thread(&source, None, 1, usize::MAX, 20_000).await.unwrap();

    assert_eq!(out["story"]["id"], 1);
    assert_eq!(out["comments_included"], 5);
    assert_eq!(out["comments"][0]["text"], "comment 10");
    assert_eq!(out["comments"][0]["by"], "user10");
    assert_eq!(out["comments"][0]["replies"][0]["replies"][0]["id"], 12);
    assert_eq!(out["comments"][1]["id"], 20);
    assert_eq!(out["article"], Value::Null);
    assert_eq!(out["article_error"], Value::Null);

    let flat: Vec<Comment> = [10, 11, 12, 13, 20]
        .iter()
        .zip([0, 1, 2, 1, 0])
        .map(|(&id, depth)| node(id, depth, vec![], vec![]).comment)
        .collect();
    let expected = build_prompt(&story(1, 0), &flat, None, usize::MAX, 20_000);
    assert_eq!(out["text"], expected);
    assert!(
        expected.contains("\n    user12: comment 12\n"),
        "{expected}"
    );
}

#[tokio::test]
async fn max_comments_keeps_the_first_in_reading_order() {
    let source = InMemorySource::new(vec![story(1, 0)]).with_comments(1, discussion());

    let out = thread(&source, None, 1, 3, 20_000).await.unwrap();

    assert_eq!(out["comments_included"], 3);
    assert_eq!(out["comments"].as_array().unwrap().len(), 1);
    assert_eq!(out["comments"][0]["replies"][0]["replies"][0]["id"], 12);
    assert_eq!(out["comments"][0]["replies"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn replies_the_source_has_not_loaded_are_fetched() {
    let root = node(10, 0, vec![11], vec![]);
    let source = InMemorySource::new(vec![story(1, 0)])
        .with_comments(1, vec![root])
        .with_children(vec![node(11, 1, vec![], vec![])]);

    let out = thread(&source, None, 1, usize::MAX, 20_000).await.unwrap();

    assert_eq!(out["comments"][0]["replies"][0]["id"], 11);
    assert_eq!(out["comments_included"], 2);
}

#[tokio::test]
async fn a_failed_article_fetch_is_reported_not_dropped() {
    let source = InMemorySource::new(vec![story(1, 0)]).with_comments(1, discussion());

    let out = thread(&source, Some(&missing_bin()), 1, usize::MAX, 20_000)
        .await
        .unwrap();

    assert_eq!(out["article"], Value::Null);
    let error = out["article_error"].as_str().unwrap();
    assert!(error.contains("not found"), "{error}");
    assert!(!out["text"].as_str().unwrap().contains("## Article"));
}

#[tokio::test]
async fn a_self_post_body_is_the_article_without_a_subprocess() {
    let mut ask = story(1, 0);
    ask.url = None;
    let source = InMemorySource::new(vec![ask]).with_thread_text(1, "<p>the body");

    let out = thread(&source, Some(&missing_bin()), 1, usize::MAX, 20_000)
        .await
        .unwrap();

    assert_eq!(out["body"], "the body");
    assert_eq!(out["article"]["content"], "the body");
    assert_eq!(out["article_error"], Value::Null);
    assert_eq!(out["text"], "# story 1\n\n## Article\n\nthe body\n\n");
}

#[tokio::test]
async fn an_unknown_story_is_not_found() {
    let source = InMemorySource::new(vec![]);

    let error = thread(&source, None, 7, usize::MAX, 20_000)
        .await
        .unwrap_err();

    assert_eq!(error.kind.exit_code(), 1);
    assert_eq!(
        error.to_json(),
        json!({"error": {"kind": "not_found", "message": "item 7 not found", "retryable": false}})
    );
}

#[test]
fn errors_are_classified_by_cause() {
    let not_story = anyhow::Error::from(StoryLookupError::NotAStory {
        id: 3,
        kind: "comment".to_string(),
    })
    .context("look up 3");
    assert_eq!(CliError::from_source(&not_story).kind.as_str(), "not_found");
    assert_eq!(
        CliError::from_source(&anyhow::anyhow!("bad JSON"))
            .kind
            .as_str(),
        "service"
    );
    let usage = CliError::usage("nope");
    assert_eq!((usage.kind.exit_code(), usage.retryable), (2, false));
}

#[test]
fn an_uncreatable_article_cache_directory_is_fatal() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let file = temp.path().join("not-a-directory");
    std::fs::write(&file, "occupied").expect("create blocking file");
    let cache = file.join("cache");
    let cli = Cli::try_parse_from(["hntui", "--file-cache-dir", cache.to_str().unwrap()]).unwrap();

    let error = cli
        .article_fetcher(&Config::for_test(temp.path().join("config.toml")))
        .expect_err("a file cannot contain the article cache");

    assert!(
        error.to_string().contains("create article cache dir"),
        "unexpected error: {error:#}"
    );
}
