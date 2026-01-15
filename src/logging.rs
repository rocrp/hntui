use anyhow::{anyhow, Context, Result};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

static LOG_FILE: OnceLock<Mutex<File>> = OnceLock::new();
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn init() -> Result<PathBuf> {
    let path = PathBuf::from("/tmp/hntui.log");
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open log file {}", path.display()))?;

    LOG_FILE
        .set(Mutex::new(file))
        .map_err(|_| anyhow!("log already initialized"))?;
    LOG_PATH
        .set(path.clone())
        .map_err(|_| anyhow!("log already initialized"))?;

    log_info("log started");
    Ok(path)
}

pub fn log_path() -> Option<&'static Path> {
    LOG_PATH.get().map(|p| p.as_path())
}

pub fn log_error(message: impl AsRef<str>) {
    log_line("ERROR", message.as_ref());
}

pub fn log_info(message: impl AsRef<str>) {
    log_line("INFO", message.as_ref());
}

fn log_line(level: &str, message: &str) {
    let file = LOG_FILE.get().expect("log not initialized");
    let mut file = file.lock().expect("log mutex poisoned");
    let ts = unix_ts();
    let line = format!("{ts} {level} {message}\n");
    file.write_all(line.as_bytes()).expect("write log");
}

fn unix_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_secs()
}
