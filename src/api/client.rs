use crate::api::file_cache::{CacheHit, FileCache};
use crate::api::types::{Comment, CommentNode, HnItem, Story};
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
use tokio::sync::{broadcast, Mutex};

#[derive(Debug, Clone)]
pub struct DiskCacheConfig {
    pub dir: PathBuf,
    pub ttl: Duration,
}

#[derive(Clone)]
pub struct HnClient {
    base_url: String,
    http: Client,
    cache: Arc<Mutex<LruCache<u64, HnItem>>>,
    file_cache: Option<Arc<FileCache>>,
    concurrency: usize,
    in_flight: Arc<Mutex<HashMap<u64, broadcast::Sender<Result<HnItem, String>>>>>,
    story_ids_cache: Arc<Mutex<Option<(Vec<u64>, Instant)>>>,
}

const COMMENT_PREFETCH_EXTRA_DEPTH: usize = 2;
const COMMENT_PREFETCH_CHILD_CONCURRENCY: usize = 8;
const STORY_IDS_CACHE_TTL: Duration = Duration::from_secs(60);
const STALE_ITEM_MAX_AGE: Duration = Duration::from_secs(60 * 60 * 24);

impl HnClient {
    pub fn new(
        base_url: String,
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
            http: Client::builder()
                .pool_max_idle_per_host(10)
                .pool_idle_timeout(Duration::from_secs(30))
                .build()
                .context("build http client")?,
            cache: Arc::new(Mutex::new(LruCache::new(cache_size))),
            file_cache,
            concurrency,
            in_flight: Arc::new(Mutex::new(HashMap::new())),
            story_ids_cache: Arc::new(Mutex::new(None)),
        })
    }

    pub async fn fetch_top_story_ids(&self) -> Result<Vec<u64>> {
        {
            let cache = self.story_ids_cache.lock().await;
            if let Some((ids, fetched_at)) = cache.as_ref() {
                if fetched_at.elapsed() < STORY_IDS_CACHE_TTL {
                    return Ok(ids.clone());
                }
            }
        }

        let ids = self.fetch_top_story_ids_network().await?;
        let mut cache = self.story_ids_cache.lock().await;
        *cache = Some((ids.clone(), Instant::now()));
        Ok(ids)
    }

    pub async fn fetch_top_story_ids_force(&self) -> Result<Vec<u64>> {
        let ids = self.fetch_top_story_ids_network().await?;
        let mut cache = self.story_ids_cache.lock().await;
        *cache = Some((ids.clone(), Instant::now()));
        Ok(ids)
    }

    async fn fetch_top_story_ids_network(&self) -> Result<Vec<u64>> {
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

    pub async fn fetch_comment_roots(&self, story: &Story) -> Result<Vec<CommentNode>> {
        if story.kids.is_empty() {
            return Ok(vec![]);
        }
        self.fetch_comment_nodes_prefetch(&story.kids, 0, COMMENT_PREFETCH_EXTRA_DEPTH)
            .await
    }

    pub async fn fetch_comment_children(
        &self,
        ids: &[u64],
        depth: usize,
    ) -> Result<Vec<CommentNode>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        self.fetch_comment_nodes_prefetch(ids, depth, COMMENT_PREFETCH_EXTRA_DEPTH)
            .await
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
        tokio::spawn(async move {
            if let Err(err) = client.fetch_item_network_deduped(id).await {
                logging::log_error(format!("failed to revalidate item id={id}: {err:#}"));
            }
        });
    }
}
