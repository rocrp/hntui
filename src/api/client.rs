use crate::api::types::{Comment, CommentNode, HnItem, Story};
use anyhow::{anyhow, Context, Result};
use futures::stream::{self, StreamExt, TryStreamExt};
use lru::LruCache;
use reqwest::Client;
use std::num::NonZeroUsize;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct HnClient {
    base_url: String,
    http: Client,
    cache: Arc<Mutex<LruCache<u64, HnItem>>>,
    concurrency: usize,
}

impl HnClient {
    pub fn new(base_url: String, cache_size: usize, concurrency: usize) -> Result<Self> {
        let cache_size = NonZeroUsize::new(cache_size).context("cache_size must be > 0")?;
        let concurrency = NonZeroUsize::new(concurrency)
            .context("concurrency must be > 0")?
            .get();

        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http: Client::new(),
            cache: Arc::new(Mutex::new(LruCache::new(cache_size))),
            concurrency,
        })
    }

    pub async fn fetch_top_story_ids(&self) -> Result<Vec<u64>> {
        let url = format!("{}/topstories.json", self.base_url);
        self.http
            .get(url)
            .send()
            .await
            .context("fetch topstories")?
            .error_for_status()
            .context("topstories status")?
            .json::<Vec<u64>>()
            .await
            .context("decode topstories")
    }

    #[allow(dead_code)]
    pub async fn fetch_top_stories(&self, count: usize) -> Result<Vec<Story>> {
        let ids = self.fetch_top_story_ids().await?;
        let ids = ids.into_iter().take(count).collect::<Vec<_>>();
        self.fetch_stories_batch(&ids).await
    }

    pub async fn fetch_item(&self, id: u64) -> Result<HnItem> {
        {
            let mut cache = self.cache.lock().await;
            if let Some(item) = cache.get(&id) {
                return Ok(item.clone());
            }
        }

        let url = format!("{}/item/{}.json", self.base_url, id);
        let item = self
            .http
            .get(url)
            .send()
            .await
            .with_context(|| format!("fetch item id={id}"))?
            .error_for_status()
            .with_context(|| format!("item status id={id}"))?
            .json::<Option<HnItem>>()
            .await
            .with_context(|| format!("decode item id={id}"))?
            .ok_or_else(|| anyhow!("item missing (null) id={id}"))?;

        let mut cache = self.cache.lock().await;
        cache.put(id, item.clone());

        Ok(item)
    }

    pub async fn fetch_items_batch(&self, ids: &[u64]) -> Result<Vec<HnItem>> {
        let concurrency = self.concurrency;

        let mut out =
            stream::iter(ids.iter().copied().enumerate())
                .map(|(idx, id)| async move {
                    Ok::<_, anyhow::Error>((idx, self.fetch_item(id).await?))
                })
                .buffer_unordered(concurrency)
                .try_collect::<Vec<_>>()
                .await?;

        out.sort_by_key(|(idx, _)| *idx);
        Ok(out.into_iter().map(|(_, item)| item).collect())
    }

    pub async fn fetch_stories_batch(&self, ids: &[u64]) -> Result<Vec<Story>> {
        self.fetch_items_batch(ids)
            .await?
            .into_iter()
            .map(Story::try_from)
            .collect()
    }

    pub async fn fetch_comments(&self, story: &Story) -> Result<Vec<CommentNode>> {
        if story.kids.is_empty() {
            return Ok(vec![]);
        }
        self.fetch_comment_nodes(&story.kids, 0).await
    }

    async fn fetch_comment_nodes(&self, ids: &[u64], depth: usize) -> Result<Vec<CommentNode>> {
        let items = self.fetch_items_batch(ids).await?;

        let mut nodes = Vec::with_capacity(items.len());
        let mut child_fetches = Vec::new();

        for (idx, item) in items.into_iter().enumerate() {
            let kids = item.kids.clone().unwrap_or_default();
            let comment = Comment::from_item(item, depth);
            let node = CommentNode {
                comment,
                children: vec![],
            };
            nodes.push(node);

            if !kids.is_empty() {
                child_fetches.push((idx, kids));
            }
        }

        let concurrency = self.concurrency;
        let fetched_children = stream::iter(child_fetches.into_iter())
            .map(|(idx, kids)| async move {
                let children = self.fetch_comment_nodes(&kids, depth + 1).await?;
                Ok::<_, anyhow::Error>((idx, children))
            })
            .buffer_unordered(concurrency)
            .try_collect::<Vec<_>>()
            .await?;

        for (idx, children) in fetched_children {
            nodes[idx].children = children;
        }

        Ok(nodes)
    }
}
