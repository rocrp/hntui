mod api;
mod app;
mod input;
mod logging;
mod plugin;
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
    #[arg(long, default_value_t = 1000)]
    pub cache_size: usize,

    /// Max simultaneous HTTP requests.
    #[arg(long, default_value_t = 8)]
    pub concurrency: usize,

    /// Disable the on-disk cache (items + story list state).
    #[arg(long, default_value_t = false)]
    pub no_file_cache: bool,

    /// Directory for the on-disk item cache (defaults to OS cache dir).
    #[arg(long)]
    pub file_cache_dir: Option<PathBuf>,

    /// Log file path (disabled by default).
    #[arg(long)]
    pub log_file: Option<PathBuf>,

    /// Max age for cached items (seconds).
    #[arg(long, default_value_t = 3600)]
    pub file_cache_ttl_secs: u64,

    /// Hacker News API base URL.
    #[arg(long, default_value = "https://hacker-news.firebaseio.com/v0")]
    pub base_url: String,

    /// UI config file path (optional; will search defaults).
    #[arg(long)]
    pub ui_config: Option<PathBuf>,

    /// Plugin config file path (optional; will search defaults).
    #[arg(long)]
    pub plugin_config: Option<PathBuf>,
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
        if let Some(path) = &self.log_file {
            anyhow::ensure!(!path.as_os_str().is_empty(), "--log-file must be non-empty");
        }
        if let Some(path) = &self.plugin_config {
            anyhow::ensure!(
                !path.as_os_str().is_empty(),
                "--plugin-config must be non-empty"
            );
        }
        Ok(())
    }
}

fn config_candidates(cli_override: Option<&PathBuf>, filename: &str) -> Vec<PathBuf> {
    if let Some(path) = cli_override {
        return vec![path.clone()];
    }

    let mut candidates = Vec::new();
    let mut push = |p: PathBuf| {
        if !candidates.contains(&p) {
            candidates.push(p);
        }
    };

    push(PathBuf::from(filename));

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            push(exe_dir.join(filename));
        }
    }

    if let Some(proj) = directories::ProjectDirs::from("dev", "hntui", "hntui") {
        push(proj.config_dir().join(filename));
    }

    // Also search ~/.config/hntui/ (XDG-style)
    if let Some(home) = dirs_home() {
        push(home.join(".config").join("hntui").join(filename));
    }

    candidates
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn ui_config_candidates(cli: &Cli) -> Vec<PathBuf> {
    config_candidates(cli.ui_config.as_ref(), "ui-config.toml")
}

fn plugin_config_candidates(cli: &Cli) -> Vec<PathBuf> {
    config_candidates(cli.plugin_config.as_ref(), "plugin-config.toml")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    cli.validate()?;
    logging::init(cli.log_file.clone()).context("init logging")?;
    let ui_candidates = ui_config_candidates(&cli);
    let allow_default = cli.ui_config.is_none();
    ui::theme::init_from_candidates(&ui_candidates, allow_default)
        .with_context(|| "load ui config")?;

    let plugin_candidates = plugin_config_candidates(&cli);
    let plugin_config = plugin::config::load_plugin_config(&plugin_candidates)
        .with_context(|| "load plugin config")?;

    app::run(cli, plugin_config).await
}
