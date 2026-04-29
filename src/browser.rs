use std::env;
use std::io::{self, Write};

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};

pub enum OpenOutcome {
    Launched,
    CopiedToClipboard,
}

pub fn open_url(url: &str) -> Result<OpenOutcome> {
    if is_remote_session() {
        copy_to_local_clipboard_via_osc52(url).context("copy URL via OSC 52")?;
        Ok(OpenOutcome::CopiedToClipboard)
    } else {
        open::that(url).context("open in browser")?;
        Ok(OpenOutcome::Launched)
    }
}

fn is_remote_session() -> bool {
    env::var_os("SSH_CONNECTION").is_some()
        || env::var_os("SSH_TTY").is_some()
        || env::var_os("SSH_CLIENT").is_some()
}

fn copy_to_local_clipboard_via_osc52(text: &str) -> Result<()> {
    let encoded = STANDARD.encode(text);
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
