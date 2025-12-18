use crate::api::Story;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs;

#[derive(Debug, Clone)]
pub(crate) struct StateStore {
    path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoryListState {
    pub saved_at: i64,
    pub story_ids: Vec<u64>,
    pub stories: Vec<Story>,
}

impl StateStore {
    pub(crate) fn new(cache_dir: PathBuf) -> Self {
        Self {
            path: cache_dir.join("state.json"),
        }
    }

    pub(crate) async fn load_story_list_state(&self) -> Result<Option<StoryListState>> {
        let bytes = match fs::read(&self.path).await {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err).with_context(|| format!("read {}", self.path.display())),
        };

        let state: StoryListState = serde_json::from_slice(&bytes)
            .with_context(|| format!("decode {}", self.path.display()))?;
        Ok(Some(state))
    }

    pub(crate) async fn save_story_list_state(
        &self,
        story_ids: Vec<u64>,
        stories: Vec<Story>,
    ) -> Result<()> {
        anyhow::ensure!(
            !story_ids.is_empty(),
            "refusing to save empty story_ids state"
        );
        anyhow::ensure!(!stories.is_empty(), "refusing to save empty stories state");

        let state = StoryListState {
            saved_at: now_unix()?,
            story_ids,
            stories,
        };
        let bytes = serde_json::to_vec(&state).context("encode story list state")?;
        atomic_write(&self.path, &bytes).await?;
        Ok(())
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
    let parent = path
        .parent()
        .context("atomic_write path has no parent dir")?;
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("create dir {}", parent.display()))?;

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
