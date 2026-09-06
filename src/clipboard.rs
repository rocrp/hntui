//! The one seam every copy in the app goes through.
//!
//! A remote session cannot reach the user's clipboard through the OS — the
//! machine running hntui is not the machine holding the clipboard — so text is
//! handed to the terminal via OSC 52 instead, exactly as a URL already is.
//! Keeping that decision here is why `y`, the Summary copy, the Article copy
//! and the Handoff cannot drift apart on it.

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::env;
use std::io::{self, Write};

pub trait Clipboard: Send + Sync {
    fn copy(&self, text: &str) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct SystemClipboard;

impl Clipboard for SystemClipboard {
    fn copy(&self, text: &str) -> Result<()> {
        if is_remote_session() {
            return copy_via_osc52(text).context("copy via OSC 52");
        }
        copy_locally(text)
    }
}

#[cfg(not(target_os = "android"))]
fn copy_locally(text: &str) -> Result<()> {
    let mut clipboard = arboard::Clipboard::new().context("open clipboard")?;
    clipboard
        .set_text(text.to_string())
        .context("copy to clipboard")
}

#[cfg(target_os = "android")]
fn copy_locally(_text: &str) -> Result<()> {
    anyhow::bail!("clipboard unavailable on Android")
}

pub(crate) fn is_remote_session() -> bool {
    env::var_os("SSH_CONNECTION").is_some()
        || env::var_os("SSH_TTY").is_some()
        || env::var_os("SSH_CLIENT").is_some()
}

/// Terminals cap the OSC 52 string and drop anything longer without a word, so
/// an oversized copy is refused rather than silently vanishing while the UI
/// flashes "Copied!". Sized to clear any summary; a whole article is what the
/// Handoff is for, and it moves the document without the terminal in the way.
const MAX_OSC52_ENCODED: usize = 100_000;

pub(crate) fn copy_via_osc52(text: &str) -> Result<()> {
    let encoded = STANDARD.encode(text);
    anyhow::ensure!(
        encoded.len() <= MAX_OSC52_ENCODED,
        "{} KB is too large for a remote clipboard; hand it off with H instead",
        text.len() / 1024
    );
    let osc52 = format!("\x1b]52;c;{encoded}\x07");
    let payload = if env::var_os("TMUX").is_some() {
        // tmux DCS passthrough: \ePtmux;<inner with each ESC doubled>\e\\
        // Requires `set -g allow-passthrough on` in tmux 3.3+.
        let escaped = osc52.replace('\x1b', "\x1b\x1b");
        format!("\x1bPtmux;{escaped}\x1b\\")
    } else {
        osc52
    };
    let mut stdout = io::stdout().lock();
    stdout.write_all(payload.as_bytes())?;
    stdout.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_oversized_copy_is_refused_instead_of_silently_dropped() {
        let huge = "x".repeat(200_000);

        let error = copy_via_osc52(&huge).expect_err("a 200 KB copy cannot fit OSC 52");

        assert_eq!(
            error.to_string(),
            "195 KB is too large for a remote clipboard; hand it off with H instead"
        );
    }

    #[test]
    fn a_summary_sized_copy_still_fits() {
        // 60 KB of text is ~80 KB encoded: comfortably inside the cap.
        let summary = "x".repeat(60_000);

        assert!(STANDARD.encode(&summary).len() <= MAX_OSC52_ENCODED);
    }
}

/// Records what was copied instead of touching a real clipboard, and can be
/// told to fail so callers' error paths are testable.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct RecordingClipboard {
    copied: std::sync::Mutex<Vec<String>>,
    fails_with: Option<String>,
}

#[cfg(test)]
impl RecordingClipboard {
    pub(crate) fn failing(message: &str) -> Self {
        Self {
            copied: std::sync::Mutex::new(Vec::new()),
            fails_with: Some(message.to_string()),
        }
    }

    pub(crate) fn copied(&self) -> Vec<String> {
        self.copied
            .lock()
            .expect("recording clipboard lock poisoned")
            .clone()
    }

    /// The single copied text, for the common case of asserting on one copy.
    pub(crate) fn last_copied(&self) -> Option<String> {
        self.copied().last().cloned()
    }
}

#[cfg(test)]
impl Clipboard for RecordingClipboard {
    fn copy(&self, text: &str) -> Result<()> {
        if let Some(message) = &self.fails_with {
            anyhow::bail!(message.clone());
        }
        self.copied
            .lock()
            .expect("recording clipboard lock poisoned")
            .push(text.to_string());
        Ok(())
    }
}
