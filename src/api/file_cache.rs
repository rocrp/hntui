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

    pub(crate) async fn get_item(&self, id: u64) -> Result<Option<HnItem>> {
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
        let age_secs = now.saturating_sub(cached.fetched_at).max(0);
        if age_secs as u64 > self.ttl.as_secs() {
            return Ok(None);
        }
        Ok(Some(cached.item))
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
}

fn now_unix() -> Result<i64> {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time before unix epoch")?;
    Ok(dur
        .as_secs()
        .try_into()
        .context("unix seconds overflow i64")?)
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
        Err(err) => Err(err).with_context(|| format!("rename {} -> {}", tmp_path.display(), path.display())),
    }
}

