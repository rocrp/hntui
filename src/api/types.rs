use anyhow::{anyhow, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

/// Which HN API backend to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum ApiBackend {
    /// node-hnapi (api.hackerwebapp.com) — pre-assembled responses, fewer requests.
    #[default]
    #[value(name = "hackerweb")]
    HackerWeb,
    /// Official Firebase API (hacker-news.firebaseio.com/v0) — item-level requests.
    Firebase,
}

/// Which feed to display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FeedKind {
    #[default]
    Top,
    New,
    Best,
    Ask,
    Show,
    Jobs,
}

impl FeedKind {
    pub const ALL: [FeedKind; 6] = [
        FeedKind::Top,
        FeedKind::New,
        FeedKind::Best,
        FeedKind::Ask,
        FeedKind::Show,
        FeedKind::Jobs,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Top => "Top Stories",
            Self::New => "New Stories",
            Self::Best => "Best Stories",
            Self::Ask => "Ask HN",
            Self::Show => "Show HN",
            Self::Jobs => "Jobs",
        }
    }

    pub fn hackerweb_path(self) -> &'static str {
        match self {
            Self::Top => "/news",
            Self::New => "/newest",
            Self::Best => "/best",
            Self::Ask => "/ask",
            Self::Show => "/show",
            Self::Jobs => "/jobs",
        }
    }

    pub fn firebase_path(self) -> &'static str {
        match self {
            Self::Top => "/topstories.json",
            Self::New => "/newstories.json",
            Self::Best => "/beststories.json",
            Self::Ask => "/askstories.json",
            Self::Show => "/showstories.json",
            Self::Jobs => "/jobstories.json",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::New => "new",
            Self::Best => "best",
            Self::Ask => "ask",
            Self::Show => "show",
            Self::Jobs => "jobs",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "top" => Some(Self::Top),
            "new" => Some(Self::New),
            "best" => Some(Self::Best),
            "ask" => Some(Self::Ask),
            "show" => Some(Self::Show),
            "jobs" => Some(Self::Jobs),
            _ => None,
        }
    }
}

// ── node-hnapi (HackerWeb) response types ──

/// A story from `/news?page=N`.
#[derive(Debug, Clone, Deserialize)]
pub struct WebStory {
    pub id: u64,
    pub title: String,
    pub url: Option<String>,
    pub points: Option<i64>,
    pub user: Option<String>,
    pub time: i64,
    pub comments_count: i64,
}

impl From<WebStory> for Story {
    fn from(ws: WebStory) -> Self {
        let internal_url = format!("item?id={}", ws.id);
        Self {
            id: ws.id,
            title: ws.title,
            // node-hnapi uses its relative discussion URL when a submission
            // has no external link. That is where HackerWeb exposes the
            // self-post body, not a page for localwebrs to visit.
            url: ws.url.filter(|url| url != &internal_url),
            // The feed listing carries no self-post body; it arrives with the
            // discussion (`/item/:id`) as a StoryThread.
            text: None,
            score: ws.points.unwrap_or(0),
            by: ws.user.unwrap_or_default(),
            time: ws.time,
            comment_count: ws.comments_count,
            kids: vec![],
        }
    }
}

/// A story with its body and nested comments from `/item/:id`.
#[derive(Debug, Clone, Deserialize)]
pub struct WebItem {
    /// Self-post body (HN HTML). Absent for link submissions.
    pub content: Option<String>,
    #[serde(default)]
    pub comments: Vec<WebComment>,
}

/// A single comment inside a `WebItem` response.
#[derive(Debug, Clone, Deserialize)]
pub struct WebComment {
    pub id: u64,
    pub user: Option<String>,
    pub time: Option<i64>,
    pub content: Option<String>,
    #[serde(default)]
    pub comments: Vec<WebComment>,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub dead: bool,
}

impl WebComment {
    /// Recursively convert into a `CommentNode` tree, filtering out dead/deleted comments.
    pub fn into_comment_node(self, depth: usize) -> Option<CommentNode> {
        if self.dead || self.deleted {
            return None;
        }

        let child_ids: Vec<u64> = self.comments.iter().map(|c| c.id).collect();
        let children: Vec<CommentNode> = self
            .comments
            .into_iter()
            .filter_map(|c| c.into_comment_node(depth + 1))
            .collect();

        let has_children = !children.is_empty();

        let text = self
            .content
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| "[no text]".to_string());

        Some(CommentNode {
            comment: Comment {
                id: self.id,
                by: self.user,
                time: self.time,
                text,
                kids: child_ids,
                depth,
                collapsed: has_children,
                children_loaded: true,
                children_loading: false,
            },
            children,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HnItemKind {
    Story,
    Comment,
    Job,
    Poll,
    Pollopt,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HnItem {
    pub id: u64,

    #[serde(rename = "type")]
    pub kind: Option<HnItemKind>,

    pub by: Option<String>,
    pub time: Option<i64>,
    pub title: Option<String>,
    pub url: Option<String>,
    pub text: Option<String>,
    pub score: Option<i64>,
    pub descendants: Option<i64>,
    pub kids: Option<Vec<u64>>,
    pub dead: Option<bool>,
    pub deleted: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Story {
    pub id: u64,
    pub title: String,
    pub url: Option<String>,
    /// Self-post body (HN HTML) — the Article for a story with no `url`.
    /// Optional in persisted state: older `state.json` files predate it.
    #[serde(default)]
    pub text: Option<String>,
    pub score: i64,
    pub by: String,
    pub time: i64,
    pub comment_count: i64,
    pub kids: Vec<u64>,
}

impl Story {
    /// Adopt a body discovered alongside the discussion, without overwriting
    /// one the listing already supplied.
    pub fn absorb_text(&mut self, text: Option<String>) {
        if self.text.is_some() {
            return;
        }
        self.text = text.filter(|text| !text.trim().is_empty());
    }
}

/// A story's discussion as one backend response: the self-post body (when the
/// backend reports one) plus the root comments.
#[derive(Debug, Clone, Default)]
pub struct StoryThread {
    pub text: Option<String>,
    pub comments: Vec<CommentNode>,
}

impl StoryThread {
    pub fn from_comments(comments: Vec<CommentNode>) -> Self {
        Self {
            text: None,
            comments,
        }
    }
}

impl TryFrom<HnItem> for Story {
    type Error = anyhow::Error;

    fn try_from(item: HnItem) -> Result<Self> {
        let kind = item.kind.unwrap_or(HnItemKind::Unknown);
        if !matches!(kind, HnItemKind::Story | HnItemKind::Job | HnItemKind::Poll) {
            return Err(anyhow!(
                "expected HN item type in [story, job, poll], got type={kind:?} id={}",
                item.id
            ));
        }

        Ok(Self {
            id: item.id,
            title: item
                .title
                .ok_or_else(|| anyhow!("item missing title id={}", item.id))?,
            url: item.url,
            text: item.text,
            score: item
                .score
                .ok_or_else(|| anyhow!("item missing score id={}", item.id))?,
            by: item
                .by
                .ok_or_else(|| anyhow!("item missing by id={}", item.id))?,
            time: item
                .time
                .ok_or_else(|| anyhow!("item missing time id={}", item.id))?,
            comment_count: item.descendants.unwrap_or(0),
            kids: item.kids.unwrap_or_default(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct Comment {
    pub id: u64,
    pub by: Option<String>,
    pub time: Option<i64>,
    pub text: String,
    pub kids: Vec<u64>,
    pub depth: usize,
    pub collapsed: bool,
    pub children_loaded: bool,
    pub children_loading: bool,
}

impl Comment {
    pub fn from_item(item: HnItem, depth: usize) -> Self {
        let kids = item.kids.unwrap_or_default();
        let text = item
            .text
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| "[no text]".to_string());

        Self {
            id: item.id,
            by: item.by,
            time: item.time,
            text,
            kids: kids.clone(),
            depth,
            collapsed: !kids.is_empty(),
            children_loaded: kids.is_empty(),
            children_loading: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommentNode {
    pub comment: Comment,
    pub children: Vec<CommentNode>,
}

#[cfg(test)]
mod tests {
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
}
