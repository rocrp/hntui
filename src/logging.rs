use anyhow::{Context, Result, anyhow};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

struct LogState {
    file: Mutex<Option<File>>,
    path: Option<PathBuf>,
}

static LOG: OnceLock<LogState> = OnceLock::new();

pub fn init(log_file: Option<PathBuf>) -> Result<()> {
    let log_file = log_file.or_else(env_log_file);
    let (file, path) = match log_file {
        Some(path) => {
            let file = open_log_file_at(&path)?;
            (Some(file), Some(path))
        }
        None => (None, None),
    };

    LOG.set(LogState {
        file: Mutex::new(file),
        path,
    })
    .map_err(|_| anyhow!("log already initialized"))?;

    log_info("log started");
    Ok(())
}

pub fn log_path() -> Option<&'static Path> {
    LOG.get()?.path.as_deref()
}

pub fn log_error(message: impl AsRef<str>) {
    log_line("ERROR", message.as_ref());
}

pub fn log_info(message: impl AsRef<str>) {
    log_line("INFO", message.as_ref());
}

fn log_line(level: &str, message: &str) {
    let Some(state) = LOG.get() else {
        return;
    };
    let mut guard = state.file.lock().expect("log mutex poisoned");
    let Some(mut file) = guard.take() else {
        return;
    };
    let ts = unix_ts();
    let line = format!("{ts} {level} {message}\n");
    match file.write_all(line.as_bytes()) {
        Ok(()) => *guard = Some(file),
        Err(err) => {
            eprintln!("hntui: log write failed: {err}");
        }
    }
}

fn unix_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_secs()
}

fn env_log_file() -> Option<PathBuf> {
    let path = std::env::var("HNTUI_LOG_FILE").ok()?;
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    Some(PathBuf::from(path))
}

fn open_log_file_at(path: &Path) -> Result<File> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create log dir {}", parent.display()))?;
        }
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open log file {}", path.display()))
}
