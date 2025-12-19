mod api;
mod app;
mod input;
mod state;
mod tui;
mod ui;

use anyhow::Context;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(name = "hntui", about = "Hacker News TUI")]
pub struct Cli {
    /// Initial number of stories to load.
    #[arg(long, default_value_t = 30)]
    pub count: usize,

    /// Page size for prefetching additional stories.
    #[arg(long, default_value_t = 30)]
    pub page_size: usize,

    /// Max items kept in the in-memory LRU cache.
    #[arg(long, default_value_t = 500)]
    pub cache_size: usize,

    /// Max simultaneous HTTP requests.
    #[arg(long, default_value_t = 20)]
    pub concurrency: usize,

    /// Disable the on-disk cache (items + story list state).
    #[arg(long, default_value_t = false)]
    pub no_file_cache: bool,

    /// Directory for the on-disk item cache (defaults to OS cache dir).
    #[arg(long)]
    pub file_cache_dir: Option<PathBuf>,

    /// Max age for cached items (seconds).
    #[arg(long, default_value_t = 3600)]
    pub file_cache_ttl_secs: u64,

    /// Hacker News API base URL.
    #[arg(long, default_value = "https://hacker-news.firebaseio.com/v0")]
    pub base_url: String,

    /// UI config file path (optional; will search defaults).
    #[arg(long)]
    pub ui_config: Option<PathBuf>,
}

impl Cli {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.count > 0, "--count must be > 0");
        anyhow::ensure!(self.page_size > 0, "--page-size must be > 0");
        anyhow::ensure!(self.cache_size > 0, "--cache-size must be > 0");
        anyhow::ensure!(self.concurrency > 0, "--concurrency must be > 0");
        anyhow::ensure!(
            self.file_cache_ttl_secs > 0,
            "--file-cache-ttl-secs must be > 0"
        );
        anyhow::ensure!(
            !self.base_url.trim().is_empty(),
            "--base-url must be non-empty"
        );
        if let Some(path) = &self.ui_config {
            anyhow::ensure!(!path.as_os_str().is_empty(), "--ui-config must be non-empty");
        }
        Ok(())
    }
}

fn ui_config_candidates(cli: &Cli) -> Vec<PathBuf> {
    if let Some(path) = &cli.ui_config {
        return vec![path.clone()];
    }

    let mut candidates = Vec::new();
    let cwd = PathBuf::from("ui-config.toml");
    if !candidates.contains(&cwd) {
        candidates.push(cwd);
    }

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let exe_cfg = exe_dir.join("ui-config.toml");
            if !candidates.contains(&exe_cfg) {
                candidates.push(exe_cfg);
            }
        }
    }

    if let Some(proj) = directories::ProjectDirs::from("dev", "hntui", "hntui") {
        let cfg = proj.config_dir().join("ui-config.toml");
        if !candidates.contains(&cfg) {
            candidates.push(cfg);
        }
    }

    candidates
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    cli.validate()?;
    let ui_candidates = ui_config_candidates(&cli);
    let allow_default = cli.ui_config.is_none();
    ui::theme::init_from_candidates(&ui_candidates, allow_default)
        .with_context(|| "load ui config")?;
    app::run(cli).await
}
