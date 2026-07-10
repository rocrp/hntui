use crate::api::types::Story;
use crate::logging;
use anyhow::{Context, Result};
use reqwest::{Client, Url};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct AlgoliaResponse {
    hits: Vec<AlgoliaHit>,
}

#[derive(Debug, Deserialize)]
struct AlgoliaHit {
    #[serde(rename = "objectID")]
    object_id: String,
    title: Option<String>,
    url: Option<String>,
    points: Option<i64>,
    author: Option<String>,
    #[serde(rename = "created_at_i")]
    created_at_i: Option<i64>,
    num_comments: Option<i64>,
}

impl AlgoliaHit {
    fn into_story(self) -> Result<Option<Story>> {
        let id: u64 = self
            .object_id
            .parse()
            .with_context(|| format!("parse Algolia objectID {:?}", self.object_id))?;
        let Some(title) = self.title else {
            return Ok(None);
        };
        if title.is_empty() {
            return Ok(None);
        }
        Ok(Some(Story {
            id,
            title,
            url: self.url.filter(|u| !u.is_empty()),
            score: self.points.unwrap_or(0),
            by: self.author.unwrap_or_default(),
            time: self.created_at_i.unwrap_or(0),
            comment_count: self.num_comments.unwrap_or(0),
            kids: vec![],
        }))
    }
}

#[derive(Clone)]
pub struct SearchClient {
    http: Client,
    endpoint: Url,
}

impl SearchClient {
    pub fn new(http: Client, endpoint: &str) -> Result<Self> {
        let endpoint = Url::parse(endpoint).context("parse Algolia endpoint")?;
        Ok(Self { http, endpoint })
    }

    /// Search stories via Algolia HN Search API.
    pub async fn search_stories(&self, query: &str) -> Result<Vec<Story>> {
        logging::log_info(format!("algolia: searching query={query:?}"));

        let resp: AlgoliaResponse = self
            .http
            .get(self.endpoint.clone())
            .query(&[("query", query), ("tags", "story"), ("hitsPerPage", "30")])
            .send()
            .await
            .context("fetch algolia search")?
            .error_for_status()
            .context("algolia search status")?
            .json()
            .await
            .context("decode algolia search")?;

        let mut stories = Vec::new();
        for hit in resp.hits {
            if let Some(story) = hit.into_story()? {
                stories.push(story);
            }
        }

        Ok(stories)
    }
}
