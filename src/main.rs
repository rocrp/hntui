mod api;
mod app;
mod browser;
mod input;
mod logging;
mod plugin;
mod state;
mod tui;
mod ui;

use crate::api::ApiBackend;
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

    /// API backend: "hackerweb" (default, faster) or "firebase" (official).
    #[arg(long, default_value = "hackerweb")]
    pub api_backend: String,

    /// Hacker News API base URL (auto-set from --api-backend if omitted).
    #[arg(long)]
    pub base_url: Option<String>,

    /// Plugin config file path (optional; will search defaults).
    #[arg(long)]
    pub plugin_config: Option<PathBuf>,

    /// Env file to load before startup. Existing process env wins.
    /// If omitted, `~/.env.smolllm` is auto-loaded when present.
    #[arg(long)]
    pub env_file: Option<PathBuf>,
}

impl Cli {
    pub fn resolved_backend(&self) -> anyhow::Result<ApiBackend> {
        self.api_backend.parse()
    }

    pub fn resolved_base_url(&self, backend: ApiBackend) -> String {
        if let Some(url) = &self.base_url {
            return url.clone();
        }
        match backend {
            ApiBackend::HackerWeb => "https://api.hackerwebapp.com".to_string(),
            ApiBackend::Firebase => "https://hacker-news.firebaseio.com/v0".to_string(),
        }
    }
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
        // Validate api_backend parses correctly.
        self.resolved_backend()?;
        if let Some(url) = &self.base_url {
            anyhow::ensure!(!url.trim().is_empty(), "--base-url must be non-empty");
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
        if let Some(path) = &self.env_file {
            anyhow::ensure!(!path.as_os_str().is_empty(), "--env-file must be non-empty");
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

/// Load env vars from a file (process env always wins).
///
/// `--env-file` is loaded explicitly and fails loudly if missing.
/// Otherwise, `~/.env.smolllm` is auto-loaded when present.
fn load_env_file(explicit: Option<&std::path::Path>) -> anyhow::Result<()> {
    if let Some(path) = explicit {
        dotenvy::from_filename(path)
            .with_context(|| format!("load --env-file {}", path.display()))?;
        return Ok(());
    }
    if let Some(home) = dirs_home() {
        let default_path = home.join(".env.smolllm");
        if default_path.exists() {
            let _ = dotenvy::from_filename(&default_path);
        }
    }
    Ok(())
}

fn plugin_config_candidates(cli: &Cli) -> Vec<PathBuf> {
    if cli.plugin_config.is_some() {
        return config_candidates(cli.plugin_config.as_ref(), "plugin-config.toml");
    }
    // Search config.toml first, then legacy plugin-config.toml for migration
    let mut candidates = config_candidates(None, "config.toml");
    candidates.extend(config_candidates(None, "plugin-config.toml"));
    candidates
}

/// Resolve path to write config to: CLI override → first existing candidate → default XDG path.
fn resolve_config_save_path(cli: &Cli) -> Option<PathBuf> {
    if let Some(path) = &cli.plugin_config {
        return Some(path.clone());
    }
    let candidates = config_candidates(None, "config.toml");
    for c in &candidates {
        if c.exists() {
            return Some(c.clone());
        }
    }
    plugin::config::default_config_path()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    cli.validate()?;
    load_env_file(cli.env_file.as_deref())?;
    logging::init(cli.log_file.clone()).context("init logging")?;
    logging::init_log_bridge();

    let plugin_candidates = plugin_config_candidates(&cli);
    let plugin_config = plugin::config::load_plugin_config(&plugin_candidates)
        .with_context(|| "load plugin config")?;
    let config_path = resolve_config_save_path(&cli);

    app::run(cli, plugin_config, config_path).await
}
