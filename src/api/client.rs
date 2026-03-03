use crate::api::file_cache::{CacheHit, FileCache};
use crate::api::types::{ApiBackend, Comment, CommentNode, FeedKind, HnItem, Story, WebItem, WebStory};
use crate::logging;
use anyhow::{anyhow, Context, Result};
use futures::stream::{self, StreamExt, TryStreamExt};
use lru::LruCache;
use reqwest::Client;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, Mutex, Semaphore};

#[derive(Debug, Clone)]
pub struct DiskCacheConfig {
    pub dir: PathBuf,
    pub ttl: Duration,
}

#[derive(Clone)]
pub struct HnClient {
    base_url: String,
    backend: ApiBackend,
    http: Client,
    cache: Arc<Mutex<LruCache<u64, HnItem>>>,
    file_cache: Option<Arc<FileCache>>,
    concurrency: usize,
    revalidate_semaphore: Arc<Semaphore>,
    in_flight: Arc<Mutex<HashMap<u64, broadcast::Sender<Result<HnItem, String>>>>>,
    story_ids_cache: Arc<Mutex<HashMap<FeedKind, (Vec<u64>, Instant)>>>,
}

const COMMENT_PREFETCH_EXTRA_DEPTH: usize = 2;
const COMMENT_PREFETCH_CHILD_CONCURRENCY: usize = 8;
const REVALIDATE_CONCURRENCY: usize = 4;
#[allow(dead_code)]
const STORY_IDS_CACHE_TTL: Duration = Duration::from_secs(60);
const STALE_ITEM_MAX_AGE: Duration = Duration::from_secs(60 * 60 * 24);

impl HnClient {
    pub fn new(
        base_url: String,
        backend: ApiBackend,
        cache_size: usize,
        concurrency: usize,
        disk_cache: Option<DiskCacheConfig>,
    ) -> Result<Self> {
        let cache_size = NonZeroUsize::new(cache_size).context("cache_size must be > 0")?;
        let concurrency = NonZeroUsize::new(concurrency)
            .context("concurrency must be > 0")?
            .get();

        let file_cache = disk_cache
            .map(|cfg| Ok::<_, anyhow::Error>(Arc::new(FileCache::new(cfg.dir, cfg.ttl)?)))
            .transpose()?;

        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            backend,
            http: Client::builder()
                .pool_max_idle_per_host(10)
                .pool_idle_timeout(Duration::from_secs(30))
                .build()
                .context("build http client")?,
            cache: Arc::new(Mutex::new(LruCache::new(cache_size))),
            file_cache,
            concurrency,
            revalidate_semaphore: Arc::new(Semaphore::new(REVALIDATE_CONCURRENCY)),
            in_flight: Arc::new(Mutex::new(HashMap::new())),
            story_ids_cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    #[allow(dead_code)]
    pub async fn fetch_story_ids(&self, feed: FeedKind) -> Result<Vec<u64>> {
        {
            let cache = self.story_ids_cache.lock().await;
            if let Some((ids, fetched_at)) = cache.get(&feed) {
                if fetched_at.elapsed() < STORY_IDS_CACHE_TTL {
                    return Ok(ids.clone());
                }
            }
        }

        let ids = self.fetch_story_ids_network(feed).await?;
        let mut cache = self.story_ids_cache.lock().await;
        cache.insert(feed, (ids.clone(), Instant::now()));
        Ok(ids)
    }

    pub async fn fetch_story_ids_force(&self, feed: FeedKind) -> Result<Vec<u64>> {
        let ids = self.fetch_story_ids_network(feed).await?;
        let mut cache = self.story_ids_cache.lock().await;
        cache.insert(feed, (ids.clone(), Instant::now()));
        Ok(ids)
    }

    async fn fetch_story_ids_network(&self, feed: FeedKind) -> Result<Vec<u64>> {
        let url = format!("{}{}", self.base_url, feed.firebase_path());
        let label = feed.as_str();
        self.http
            .get(url)
            .send()
            .await
            .with_context(|| format!("fetch {label} stories"))?
            .error_for_status()
            .with_context(|| format!("{label} stories status"))?
            .json::<Vec<u64>>()
            .await
            .with_context(|| format!("decode {label} stories"))
    }

    pub fn cleanup_disk_cache_background(&self, max_age: Duration) {
        let Some(file_cache) = self.file_cache.clone() else {
            return;
        };
        tokio::spawn(async move {
            match file_cache.cleanup_expired(max_age).await {
                Ok(removed) => {
                    if removed > 0 {
                        logging::log_info(format!("cleaned {removed} expired cache entries"));
                    }
                }
                Err(err) => {
                    logging::log_error(format!("failed to cleanup cache: {err:#}"));
                }
            }
        });
    }

    // ── Unified public methods (backend-aware) ──

    /// Fetch initial stories. Returns `(story_ids, stories)`.
    ///
    /// - **HackerWeb**: fetches pre-assembled pages (page 1 + page 2 if count > 30).
    /// - **Firebase**: fetches story IDs then batch-fetches items.
    pub async fn fetch_initial_stories(&self, feed: FeedKind, count: usize) -> Result<(Vec<u64>, Vec<Story>)> {
        match self.backend {
            ApiBackend::HackerWeb => {
                let mut stories = self.fetch_hackerweb_feed(feed, 1).await?;
                if count > 30 && stories.len() == 30 {
                    let page2 = self.fetch_hackerweb_feed(feed, 2).await?;
                    stories.extend(page2);
                }
                stories.truncate(count);
                let ids: Vec<u64> = stories.iter().map(|s| s.id).collect();
                Ok((ids, stories))
            }
            ApiBackend::Firebase => {
                let story_ids = self.fetch_story_ids_force(feed).await?;
                let ids: Vec<u64> = story_ids.iter().copied().take(count).collect();
                let stories = self.fetch_stories_batch(&ids).await?;
                Ok((story_ids, stories))
            }
        }
    }

    /// Fetch more stories beyond what's already loaded.
    ///
    /// - **HackerWeb**: computes next page number, fetches it.
    /// - **Firebase**: uses stored story_ids to batch-fetch the next slice.
    pub async fn fetch_more_stories(
        &self,
        feed: FeedKind,
        story_ids: &[u64],
        loaded_count: usize,
        page_size: usize,
    ) -> Result<Vec<Story>> {
        match self.backend {
            ApiBackend::HackerWeb => {
                let page = (loaded_count / 30) + 2; // page 1 already loaded, so next is 2, etc.
                self.fetch_hackerweb_feed(feed, page).await
            }
            ApiBackend::Firebase => {
                if loaded_count >= story_ids.len() {
                    return Ok(vec![]);
                }
                let end = (loaded_count + page_size).min(story_ids.len());
                let ids = &story_ids[loaded_count..end];
                self.fetch_stories_batch(ids).await
            }
        }
    }

    /// Fetch root-level comments for a story.
    ///
    /// - **HackerWeb**: single request to `/item/:id`, returns full nested tree.
    /// - **Firebase**: recursive item fetches with prefetch depth.
    pub async fn fetch_comment_roots(&self, story: &Story) -> Result<Vec<CommentNode>> {
        match self.backend {
            ApiBackend::HackerWeb => self.fetch_hackerweb_comments(story.id).await,
            ApiBackend::Firebase => {
                let kids = if story.kids.is_empty() && story.comment_count > 0 {
                    // Search results don't include kids — fetch the item to get them.
                    let item = self.fetch_item(story.id).await?;
                    item.kids.unwrap_or_default()
                } else {
                    story.kids.clone()
                };
                if kids.is_empty() {
                    return Ok(vec![]);
                }
                self.fetch_comment_nodes_prefetch(&kids, 0, COMMENT_PREFETCH_EXTRA_DEPTH)
                    .await
            }
        }
    }

    /// Fetch children of a comment for lazy expand.
    ///
    /// - **HackerWeb**: all children are pre-loaded; returns empty vec as safety fallback.
    /// - **Firebase**: recursive item fetches.
    pub async fn fetch_comment_children(
        &self,
        ids: &[u64],
        depth: usize,
    ) -> Result<Vec<CommentNode>> {
        match self.backend {
            ApiBackend::HackerWeb => Ok(vec![]),
            ApiBackend::Firebase => {
                if ids.is_empty() {
                    return Ok(vec![]);
                }
                self.fetch_comment_nodes_prefetch(ids, depth, COMMENT_PREFETCH_EXTRA_DEPTH)
                    .await
            }
        }
    }

    // ── HackerWeb private methods ──

    async fn fetch_hackerweb_feed(&self, feed: FeedKind, page: usize) -> Result<Vec<Story>> {
        let path = feed.hackerweb_path();
        let url = format!("{}{}?page={page}", self.base_url, path);
        let label = feed.as_str();
        logging::log_info(format!("hackerweb: fetching {url}"));
        let web_stories: Vec<WebStory> = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("fetch hackerweb {label} page={page}"))?
            .error_for_status()
            .with_context(|| format!("hackerweb {label} status page={page}"))?
            .json()
            .await
            .with_context(|| format!("decode hackerweb {label} page={page}"))?;
        Ok(web_stories.into_iter().map(Story::from).collect())
    }

    async fn fetch_hackerweb_comments(&self, story_id: u64) -> Result<Vec<CommentNode>> {
        let url = format!("{}/item/{story_id}", self.base_url);
        logging::log_info(format!("hackerweb: fetching {url}"));
        let web_item: WebItem = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("fetch hackerweb item id={story_id}"))?
            .error_for_status()
            .with_context(|| format!("hackerweb item status id={story_id}"))?
            .json()
            .await
            .with_context(|| format!("decode hackerweb item id={story_id}"))?;
        Ok(web_item
            .comments
            .into_iter()
            .map(|c| c.into_comment_node(0))
            .collect())
    }

    // ── Firebase-only methods (kept for Firebase backend) ──

    pub async fn fetch_item(&self, id: u64) -> Result<HnItem> {
        {
            let mut cache = self.cache.lock().await;
            if let Some(item) = cache.get(&id) {
                return Ok(item.clone());
            }
        }

        if let Some(file_cache) = &self.file_cache {
            if let Some(hit) = file_cache.get_item_with_staleness(id).await? {
                match hit {
                    CacheHit::Fresh(item) => {
                        let mut cache = self.cache.lock().await;
                        cache.put(id, item.clone());
                        return Ok(item);
                    }
                    CacheHit::Stale { item, stale_secs } => {
                        if stale_secs <= STALE_ITEM_MAX_AGE.as_secs() {
                            let mut cache = self.cache.lock().await;
                            cache.put(id, item.clone());
                            self.spawn_revalidate_item(id);
                            return Ok(item);
                        }
                    }
                }
            }
        }

        self.fetch_item_network_deduped(id).await
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

    async fn fetch_comment_nodes_prefetch(
        &self,
        ids: &[u64],
        depth: usize,
        prefetch_extra_depth: usize,
    ) -> Result<Vec<CommentNode>> {
        let items = self.fetch_items_batch(ids).await?;

        let mut nodes = Vec::with_capacity(items.len());
        for item in items {
            let comment = Comment::from_item(item, depth);
            let node = CommentNode {
                comment,
                children: vec![],
            };
            nodes.push(node);
        }

        if prefetch_extra_depth == 0 {
            return Ok(nodes);
        }

        let mut child_batches = Vec::new();
        for (idx, node) in nodes.iter().enumerate() {
            if node.comment.kids.is_empty() {
                continue;
            }
            child_batches.push((idx, node.comment.kids.clone()));
        }

        if child_batches.is_empty() {
            return Ok(nodes);
        }

        let child_concurrency = self.concurrency.min(COMMENT_PREFETCH_CHILD_CONCURRENCY);
        let child_results = stream::iter(child_batches.into_iter())
            .map(|(idx, kids)| async move {
                let children = self
                    .fetch_comment_nodes_prefetch(&kids, depth + 1, prefetch_extra_depth - 1)
                    .await?;
                Ok::<_, anyhow::Error>((idx, children))
            })
            .buffer_unordered(child_concurrency.max(1))
            .try_collect::<Vec<_>>()
            .await?;

        for (idx, children) in child_results {
            nodes[idx].children = children;
            nodes[idx].comment.children_loaded = true;
        }

        Ok(nodes)
    }

    async fn fetch_item_network_deduped(&self, id: u64) -> Result<HnItem> {
        let mut rx = {
            let mut in_flight = self.in_flight.lock().await;
            if let Some(existing) = in_flight.get(&id) {
                existing.subscribe()
            } else {
                let (tx, _rx) = broadcast::channel(1);
                in_flight.insert(id, tx.clone());
                drop(in_flight);
                let result = self.fetch_item_network(id).await;
                let send_result = match &result {
                    Ok(item) => Ok(item.clone()),
                    Err(err) => Err(format!("{err:#}")),
                };
                let _ = tx.send(send_result);
                let mut in_flight = self.in_flight.lock().await;
                in_flight.remove(&id);
                return result;
            }
        };

        match rx.recv().await {
            Ok(Ok(item)) => Ok(item),
            Ok(Err(message)) => Err(anyhow!(message)),
            Err(err) => Err(anyhow!("in-flight request failed id={id}: {err}")),
        }
    }

    async fn fetch_item_network(&self, id: u64) -> Result<HnItem> {
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

        if let Some(file_cache) = &self.file_cache {
            file_cache.put_item(id, item.clone()).await?;
        }

        Ok(item)
    }

    fn spawn_revalidate_item(&self, id: u64) {
        let client = self.clone();
        let semaphore = self.revalidate_semaphore.clone();
        tokio::spawn(async move {
            let _permit = match semaphore.try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => return,
            };
            if let Err(err) = client.fetch_item_network_deduped(id).await {
                logging::log_error(format!("failed to revalidate item id={id}: {err:#}"));
            }
        });
    }
}
