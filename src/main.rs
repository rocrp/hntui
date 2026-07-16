mod api;
mod app;
mod browser;
mod config;
mod input;
mod logging;
mod state;
mod summarizer;
mod tasks;
mod text;
mod tui;
mod ui;

use crate::api::ApiBackend;
use anyhow::Context;
use clap::builder::{NonEmptyStringValueParser, TypedValueParser};
use clap::Parser;
use std::num::{NonZeroU64, NonZeroUsize};
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "hntui",
    version,
    about = "Hacker News TUI",
    help_template = "{before-help}{name} {version}\n\
{about-with-newline}\n\
{usage-heading} {usage}\n\n\
{all-args}{after-help}"
)]
pub struct Cli {
    /// Initial number of stories to load.
    #[arg(long, default_value = "30")]
    pub count: NonZeroUsize,

    /// Page size for prefetching additional stories.
    #[arg(long, default_value = "30")]
    pub page_size: NonZeroUsize,

    /// Max items kept in the in-memory LRU cache.
    #[arg(long, default_value = "1000")]
    pub cache_size: NonZeroUsize,

    /// Max simultaneous HTTP requests.
    #[arg(long, default_value = "8")]
    pub concurrency: NonZeroUsize,

    /// Disable the on-disk cache (items + story list state).
    #[arg(long, default_value_t = false)]
    pub no_file_cache: bool,

    /// Directory for the on-disk item cache (defaults to OS cache dir).
    #[arg(long, value_parser = NonEmptyStringValueParser::new().map(PathBuf::from))]
    pub file_cache_dir: Option<PathBuf>,

    /// Log file path (disabled by default).
    #[arg(long, value_parser = NonEmptyStringValueParser::new().map(PathBuf::from))]
    pub log_file: Option<PathBuf>,

    /// Max age for cached items (seconds).
    #[arg(long, default_value = "3600")]
    pub file_cache_ttl_secs: NonZeroU64,

    /// API backend: "hackerweb" (default, faster) or "firebase" (official).
    #[arg(long, value_enum, default_value = "hackerweb")]
    pub api_backend: ApiBackend,

    /// Hacker News API base URL (auto-set from --api-backend if omitted).
    #[arg(long, value_parser = parse_nonblank)]
    pub base_url: Option<String>,

    /// Config file path (searches default locations when omitted).
    #[arg(long, value_parser = NonEmptyStringValueParser::new().map(PathBuf::from))]
    pub config: Option<PathBuf>,

    /// Env file to load before startup. Existing process env wins.
    /// If omitted, `~/.env.smolllm` is auto-loaded when present.
    #[arg(long, value_parser = NonEmptyStringValueParser::new().map(PathBuf::from))]
    pub env_file: Option<PathBuf>,
}

fn parse_nonblank(value: &str) -> Result<String, &'static str> {
    if value.trim().is_empty() {
        return Err("value must not be empty");
    }
    Ok(value.to_string())
}

impl Cli {
    pub fn resolved_base_url(&self) -> String {
        if let Some(url) = &self.base_url {
            return url.clone();
        }
        match self.api_backend {
            ApiBackend::HackerWeb => "https://api.hackerwebapp.com".to_string(),
            ApiBackend::Firebase => "https://hacker-news.firebaseio.com/v0".to_string(),
        }
    }
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    load_env_file(cli.env_file.as_deref())?;
    logging::init(cli.log_file.clone()).context("init logging")?;
    logging::init_log_bridge();

    let config = config::Config::load(cli.config.as_deref()).context("load config")?;

    app::run(cli, config).await
}
