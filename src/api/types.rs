use anyhow::{anyhow, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct HnItem {
    pub id: u64,

    #[serde(rename = "type")]
    pub kind: Option<String>,

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

#[derive(Debug, Clone)]
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
        let kind = item.kind.as_deref().unwrap_or("");
        if kind != "story" {
            return Err(anyhow!(
                "expected HN item type=story, got type={kind:?} id={}",
                item.id
            ));
        }

        Ok(Self {
            id: item.id,
            title: item
                .title
                .ok_or_else(|| anyhow!("story missing title id={}", item.id))?,
            url: item.url,
            score: item
                .score
                .ok_or_else(|| anyhow!("story missing score id={}", item.id))?,
            by: item
                .by
                .ok_or_else(|| anyhow!("story missing by id={}", item.id))?,
            time: item
                .time
                .ok_or_else(|| anyhow!("story missing time id={}", item.id))?,
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
    pub deleted: bool,
    pub dead: bool,
}

impl Comment {
    pub fn from_item(item: HnItem, depth: usize) -> Self {
        let deleted = item.deleted.unwrap_or(false);
        let dead = item.dead.unwrap_or(false);
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
            kids: item.kids.unwrap_or_default(),
            depth,
            collapsed: false,
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
