use crate::clipboard::{copy_via_osc52, is_remote_session};
use anyhow::{Context, Result};

pub trait UrlOpener: Send + Sync {
    fn open(&self, url: &str) -> Result<OpenOutcome>;
}

#[derive(Debug, Default)]
pub struct SystemUrlOpener;

pub enum OpenOutcome {
    Launched,
    CopiedToClipboard,
}

impl UrlOpener for SystemUrlOpener {
    fn open(&self, url: &str) -> Result<OpenOutcome> {
        if is_remote_session() {
            copy_via_osc52(url).context("copy URL via OSC 52")?;
            Ok(OpenOutcome::CopiedToClipboard)
        } else {
            open::that(url).context("open in browser")?;
            Ok(OpenOutcome::Launched)
        }
    }
}

#[cfg(test)]
#[derive(Debug, Default)]
pub struct RecordingUrlOpener {
    opened_urls: std::sync::Mutex<Vec<String>>,
}

#[cfg(test)]
impl RecordingUrlOpener {
    pub fn opened_urls(&self) -> Vec<String> {
        self.opened_urls
            .lock()
            .expect("recording URL opener lock poisoned")
            .clone()
    }
}

#[cfg(test)]
impl UrlOpener for RecordingUrlOpener {
    fn open(&self, url: &str) -> Result<OpenOutcome> {
        self.opened_urls
            .lock()
            .expect("recording URL opener lock poisoned")
            .push(url.to_string());
        Ok(OpenOutcome::Launched)
    }
}
