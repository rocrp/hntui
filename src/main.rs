mod api;
mod app;
mod article;
mod browser;
mod cli;
mod clipboard;
mod config;
mod editor;
mod handoff;
mod input;
mod logging;
mod state;
mod summarizer;
mod tasks;
mod text;
mod tui;
mod ui;

use crate::api::{ApiBackend, DiskCacheConfig, HnClient};
use crate::article::ArticleFetcher;
use anyhow::Context;
use clap::builder::{NonEmptyStringValueParser, TypedValueParser};
use clap::{CommandFactory, Parser};
use std::io::IsTerminal;
use std::num::{NonZeroU64, NonZeroUsize};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "hntui",
    version,
    about = "Hacker News TUI",
    help_template = "{before-help}{name} {version}\n\
{about-with-newline}\n\
{usage-heading} {usage}\n\n\
{all-args}{after-help}",
    after_help = cli::AFTER_HELP
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<cli::Command>,

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
    #[arg(long)]
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

    /// `--file-cache-dir`, else the OS cache dir. Home of the HN item cache,
    /// the story list state, and localwebrs's own cache.
    pub fn cache_dir(&self) -> anyhow::Result<PathBuf> {
        if let Some(dir) = &self.file_cache_dir {
            return Ok(dir.clone());
        }
        let proj = directories::ProjectDirs::from("dev", "hntui", "hntui")
            .context("resolve OS cache dir")?;
        Ok(proj.cache_dir().to_path_buf())
    }

    /// The HN client, with the on-disk item cache unless `--no-file-cache`.
    pub fn hn_client(&self, http: reqwest::Client) -> anyhow::Result<HnClient> {
        let disk_cache = if self.no_file_cache {
            None
        } else {
            Some(DiskCacheConfig {
                dir: self.cache_dir()?,
                ttl: Duration::from_secs(self.file_cache_ttl_secs.get()),
            })
        };
        HnClient::new(
            http,
            self.resolved_base_url(),
            self.api_backend,
            self.cache_size.get(),
            self.concurrency.get(),
            disk_cache,
        )
    }

    /// localwebrs writes a CWD-relative `cache/cache.sqlite`, so it must run
    /// from our cache dir or it litters the user's working directory. The dir
    /// has to exist up front: a missing `current_dir` fails the spawn with the
    /// same ENOENT a missing binary would, which would misreport as "install
    /// localwebrs". Independent of --no-file-cache, which is about HN items.
    pub fn article_fetcher(&self, config: &config::Config) -> anyhow::Result<ArticleFetcher> {
        let dir = self.cache_dir()?;
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("create article cache dir {}", dir.display()))?;
        Ok(ArticleFetcher::new(config.article_bin(), Some(dir)))
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
    let mut cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) if !error.use_stderr() => error.exit(), // --help / --version
        Err(error) => cli::fail(&cli::CliError::usage(error.render().to_string().trim_end())),
    };
    let Some(command) = cli.command.take() else {
        if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
            eprintln!("{}", Cli::command().render_help());
            std::process::exit(2);
        }
        return run_tui(cli).await;
    };
    match run_command(&cli, command).await {
        Ok(output) => println!("{output}"),
        Err(error) => cli::fail(&error),
    }
    Ok(())
}

async fn run_tui(cli: Cli) -> anyhow::Result<()> {
    load_env_file(cli.env_file.as_deref())?;
    logging::init(cli.log_file.clone()).context("init logging")?;
    logging::init_log_bridge();
    let config = config::Config::load(cli.config.as_deref()).context("load config")?;
    app::run(cli, config).await
}

/// Setup failures here are local (env file, config, cache dir); what the
/// command itself hits is classified by `cli::run`.
async fn run_command(cli: &Cli, command: cli::Command) -> cli::Result<serde_json::Value> {
    let local = |error: anyhow::Error| cli::CliError::local(format!("{error:#}"));
    load_env_file(cli.env_file.as_deref()).map_err(local)?;
    logging::init(cli.log_file.clone())
        .context("init logging")
        .map_err(local)?;
    let config = config::Config::load(cli.config.as_deref())
        .context("load config")
        .map_err(local)?;
    let source = cli.hn_client(reqwest::Client::new()).map_err(local)?;
    let fetcher = cli.article_fetcher(&config).map_err(local)?;
    cli::run(command, &source, &fetcher, &config).await
}
