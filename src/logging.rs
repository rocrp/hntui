use anyhow::{Context, Result, anyhow};
use directories::{BaseDirs, ProjectDirs};
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

pub fn init() -> Result<()> {
    let (file, path) = match try_open_log_file() {
        Ok((file, path)) => (Some(file), Some(path)),
        Err(err) => {
            eprintln!("hntui: logging disabled: {err:#}");
            (None, None)
        }
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

fn try_open_log_file() -> Result<(File, PathBuf)> {
    let mut last_err: Option<anyhow::Error> = None;
    for path in log_file_candidates() {
        match open_log_file_at(&path) {
            Ok(file) => return Ok((file, path)),
            Err(err) => last_err = Some(err),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("no log file candidates")))
}

fn log_file_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();

    if let Ok(path) = std::env::var("HNTUI_LOG_FILE") {
        let path = path.trim();
        if !path.is_empty() {
            out.push(PathBuf::from(path));
        }
    }

    if let Some(proj) = ProjectDirs::from("dev", "hntui", "hntui") {
        out.push(proj.cache_dir().join("hntui.log"));
    }

    if let Some(base) = BaseDirs::new() {
        out.push(base.cache_dir().join("hntui/hntui.log"));
    }

    out.push(PathBuf::from("hntui.log"));
    out
}

fn open_log_file_at(path: &Path) -> Result<File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create log dir {}", parent.display()))?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open log file {}", path.display()))
}
