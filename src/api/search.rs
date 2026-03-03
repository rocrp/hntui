use crate::api::types::Story;
use crate::logging;
use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct AlgoliaResponse {
    hits: Vec<AlgoliaHit>,
    #[serde(rename = "nbPages")]
    nb_pages: u64,
    page: u64,
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
    fn into_story(self) -> Option<Story> {
        let id: u64 = self.object_id.parse().ok()?;
        let title = self.title?;
        if title.is_empty() {
            return None;
        }
        Some(Story {
            id,
            title,
            url: self.url.filter(|u| !u.is_empty()),
            score: self.points.unwrap_or(0),
            by: self.author.unwrap_or_default(),
            time: self.created_at_i.unwrap_or(0),
            comment_count: self.num_comments.unwrap_or(0),
            kids: vec![],
        })
    }
}

#[derive(Clone)]
pub struct SearchClient {
    http: Client,
}

impl SearchClient {
    pub fn new(http: Client) -> Self {
        Self { http }
    }

    /// Search stories via Algolia HN Search API.
    /// Returns `(stories, has_more_pages)`.
    pub async fn search_stories(
        &self,
        query: &str,
        page: u32,
    ) -> Result<(Vec<Story>, bool)> {
        let url = "https://hn.algolia.com/api/v1/search";
        logging::log_info(format!("algolia: searching query={query:?} page={page}"));

        let resp: AlgoliaResponse = self
            .http
            .get(url)
            .query(&[
                ("query", query),
                ("tags", "story"),
                ("hitsPerPage", "30"),
                ("page", &page.to_string()),
            ])
            .send()
            .await
            .context("fetch algolia search")?
            .error_for_status()
            .context("algolia search status")?
            .json()
            .await
            .context("decode algolia search")?;

        let has_more = resp.page + 1 < resp.nb_pages;
        let stories: Vec<Story> = resp
            .hits
            .into_iter()
            .filter_map(AlgoliaHit::into_story)
            .collect();

        Ok((stories, has_more))
    }
}
