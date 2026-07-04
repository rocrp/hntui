use crate::api::types::HnItem;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::fs;

#[derive(Debug, Clone)]
pub(crate) struct FileCache {
    items_dir: PathBuf,
    ttl: Duration,
}

#[derive(Debug, Clone)]
pub(crate) enum CacheHit {
    Fresh(HnItem),
    Stale { item: HnItem, stale_secs: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedItem {
    fetched_at: i64,
    item: HnItem,
}

impl FileCache {
    pub(crate) fn new(dir: PathBuf, ttl: Duration) -> Result<Self> {
        anyhow::ensure!(ttl.as_secs() > 0, "file cache ttl must be > 0s");
        let items_dir = dir.join("items");
        std::fs::create_dir_all(&items_dir)
            .with_context(|| format!("create cache dir {}", items_dir.display()))?;
        Ok(Self { items_dir, ttl })
    }

    pub(crate) async fn get_item_with_staleness(&self, id: u64) -> Result<Option<CacheHit>> {
        let path = self.item_path(id);
        let bytes = match fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(err).with_context(|| format!("read cache {}", path.display()));
            }
        };

        let cached: CachedItem = serde_json::from_slice(&bytes)
            .with_context(|| format!("decode cache {}", path.display()))?;
        let now = now_unix()?;
        let age_secs = now.saturating_sub(cached.fetched_at).max(0) as u64;
        if age_secs <= self.ttl.as_secs() {
            return Ok(Some(CacheHit::Fresh(cached.item)));
        }
        Ok(Some(CacheHit::Stale {
            item: cached.item,
            stale_secs: age_secs,
        }))
    }

    pub(crate) async fn put_item(&self, id: u64, item: HnItem) -> Result<()> {
        let path = self.item_path(id);
        let cached = CachedItem {
            fetched_at: now_unix()?,
            item,
        };
        let bytes = serde_json::to_vec(&cached).context("encode cache")?;
        atomic_write(&path, &bytes).await?;
        Ok(())
    }

    fn item_path(&self, id: u64) -> PathBuf {
        self.items_dir.join(format!("{id}.json"))
    }

    pub(crate) async fn cleanup_expired(&self, max_age: Duration) -> Result<usize> {
        anyhow::ensure!(max_age.as_secs() > 0, "max_age must be > 0s");
        let mut removed = 0usize;
        let now = now_unix()?;
        let mut entries = fs::read_dir(&self.items_dir)
            .await
            .with_context(|| format!("read cache dir {}", self.items_dir.display()))?;

        while let Some(entry) = entries.next_entry().await.context("read cache dir entry")? {
            let path = entry.path();
            let file_type = entry
                .file_type()
                .await
                .with_context(|| format!("stat cache entry {}", path.display()))?;
            if !file_type.is_file() {
                return Err(anyhow::anyhow!(
                    "unexpected non-file in cache dir {}",
                    path.display()
                ));
            }

            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .context("cache entry name not utf-8")?;
            if !file_name.ends_with(".json") {
                fs::remove_file(&path)
                    .await
                    .with_context(|| format!("remove stray cache file {}", path.display()))?;
                removed += 1;
                continue;
            }

            let bytes = fs::read(&path)
                .await
                .with_context(|| format!("read cache {}", path.display()))?;
            let cached: CachedItem = serde_json::from_slice(&bytes)
                .with_context(|| format!("decode cache {}", path.display()))?;
            let age_secs = now.saturating_sub(cached.fetched_at).max(0) as u64;
            if age_secs > max_age.as_secs() {
                fs::remove_file(&path)
                    .await
                    .with_context(|| format!("remove expired cache {}", path.display()))?;
                removed += 1;
            }
        }

        Ok(removed)
    }
}

fn now_unix() -> Result<i64> {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time before unix epoch")?;
    dur.as_secs()
        .try_into()
        .context("unix seconds overflow i64")
}

async fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time before unix epoch")?
        .as_nanos();
    let pid = std::process::id();
    let tmp_path = path.with_extension(format!("json.tmp.{pid}.{unique}"));

    fs::write(&tmp_path, bytes)
        .await
        .with_context(|| format!("write temp {}", tmp_path.display()))?;

    match fs::rename(&tmp_path, path).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            fs::remove_file(path)
                .await
                .with_context(|| format!("remove existing {}", path.display()))?;
            fs::rename(&tmp_path, path)
                .await
                .with_context(|| format!("rename {} -> {}", tmp_path.display(), path.display()))?;
            Ok(())
        }
        Err(err) => {
            Err(err).with_context(|| format!("rename {} -> {}", tmp_path.display(), path.display()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::HnItemKind;

    fn temp_cache_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("hntui-{name}-{}-{unique}", std::process::id()))
    }

    fn item(id: u64) -> HnItem {
        HnItem {
            id,
            kind: Some(HnItemKind::Story),
            by: Some("alice".to_string()),
            time: Some(1),
            title: Some(format!("story {id}")),
            url: None,
            text: None,
            score: Some(1),
            descendants: Some(0),
            kids: None,
            dead: None,
            deleted: None,
        }
    }

    #[tokio::test]
    async fn get_item_reports_fresh_and_stale_hits() {
        let dir = temp_cache_dir("fresh-stale");
        let cache = FileCache::new(dir.clone(), Duration::from_secs(60)).expect("create cache");

        cache.put_item(1, item(1)).await.expect("put fresh item");
        match cache
            .get_item_with_staleness(1)
            .await
            .expect("get fresh item")
            .expect("fresh hit")
        {
            CacheHit::Fresh(hit) => assert_eq!(hit.id, 1),
            CacheHit::Stale { .. } => panic!("expected fresh hit"),
        }

        let stale = CachedItem {
            fetched_at: now_unix().expect("now") - 120,
            item: item(2),
        };
        fs::write(
            cache.item_path(2),
            serde_json::to_vec(&stale).expect("encode stale item"),
        )
        .await
        .expect("write stale item");

        match cache
            .get_item_with_staleness(2)
            .await
            .expect("get stale item")
            .expect("stale hit")
        {
            CacheHit::Fresh(_) => panic!("expected stale hit"),
            CacheHit::Stale { item, stale_secs } => {
                assert_eq!(item.id, 2);
                assert!(stale_secs >= 120);
            }
        }

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn cleanup_expired_removes_stale_and_stray_files_only() {
        let dir = temp_cache_dir("cleanup");
        let cache = FileCache::new(dir.clone(), Duration::from_secs(60)).expect("create cache");

        cache.put_item(1, item(1)).await.expect("put fresh item");
        let stale = CachedItem {
            fetched_at: now_unix().expect("now") - 600,
            item: item(2),
        };
        fs::write(
            cache.item_path(2),
            serde_json::to_vec(&stale).expect("encode stale item"),
        )
        .await
        .expect("write stale item");
        fs::write(cache.items_dir.join("stray.txt"), b"junk")
            .await
            .expect("write stray file");

        let removed = cache
            .cleanup_expired(Duration::from_secs(300))
            .await
            .expect("cleanup");

        assert_eq!(removed, 2);
        assert!(cache.item_path(1).exists());
        assert!(!cache.item_path(2).exists());
        assert!(!cache.items_dir.join("stray.txt").exists());

        let _ = std::fs::remove_dir_all(dir);
    }
}
