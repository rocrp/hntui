use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Which HN API backend to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ApiBackend {
    /// node-hnapi (api.hackerwebapp.com) — pre-assembled responses, fewer requests.
    #[default]
    HackerWeb,
    /// Official Firebase API (hacker-news.firebaseio.com/v0) — item-level requests.
    Firebase,
}

impl fmt::Display for ApiBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HackerWeb => write!(f, "hackerweb"),
            Self::Firebase => write!(f, "firebase"),
        }
    }
}

impl FromStr for ApiBackend {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "hackerweb" | "hackerweb-api" | "hw" => Ok(Self::HackerWeb),
            "firebase" | "fb" => Ok(Self::Firebase),
            _ => Err(anyhow!(
                "unknown API backend: {s:?} (expected hackerweb or firebase)"
            )),
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
        Self {
            id: ws.id,
            title: ws.title,
            url: ws.url,
            score: ws.points.unwrap_or(0),
            by: ws.user.unwrap_or_default(),
            time: ws.time,
            comment_count: ws.comments_count,
            kids: vec![],
        }
    }
}

/// A story with nested comments from `/item/:id`.
#[derive(Debug, Clone, Deserialize)]
pub struct WebItem {
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
    /// Recursively convert into a `CommentNode` tree.
    pub fn into_comment_node(self, depth: usize) -> CommentNode {
        let child_ids: Vec<u64> = self.comments.iter().map(|c| c.id).collect();
        let children: Vec<CommentNode> = self
            .comments
            .into_iter()
            .map(|c| c.into_comment_node(depth + 1))
            .collect();

        let has_children = !children.is_empty();

        let text = self
            .content
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| {
                if self.deleted {
                    "[deleted]".to_string()
                } else if self.dead {
                    "[dead]".to_string()
                } else {
                    "[no text]".to_string()
                }
            });

        CommentNode {
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
                deleted: self.deleted,
                dead: self.dead,
            },
            children,
        }
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
    pub score: i64,
    pub by: String,
    pub time: i64,
    pub comment_count: i64,
    pub kids: Vec<u64>,
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
    pub deleted: bool,
    pub dead: bool,
}

impl Comment {
    pub fn from_item(item: HnItem, depth: usize) -> Self {
        let deleted = item.deleted.unwrap_or(false);
        let dead = item.dead.unwrap_or(false);
        let kids = item.kids.unwrap_or_default();
        let text = item
            .text
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| {
                if deleted {
                    "[deleted]".to_string()
                } else if dead {
                    "[dead]".to_string()
                } else {
                    "[no text]".to_string()
                }
            });

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
            deleted,
            dead,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommentNode {
    pub comment: Comment,
    pub children: Vec<CommentNode>,
}
