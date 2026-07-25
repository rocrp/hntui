use super::{App, AppEvent, TaskTarget, View};
use crate::api::{DiskCacheConfig, FeedKind, HnClient, SearchClient, Sources};
use crate::article::ArticleFetcher;
use crate::config::Config;
use crate::logging;
use crate::state::StateStore;
use crate::summarizer::Summarizer;
use crate::tui::Tui;
use crate::ui;
use crate::Cli;
use anyhow::{Context, Result};
use crossterm::event::{Event, EventStream};
use futures::StreamExt;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

pub async fn run(cli: Cli, config: Config) -> Result<()> {
    let resolved_cache_dir = match cli.file_cache_dir.clone() {
        Some(dir) => dir,
        None => {
            let proj = directories::ProjectDirs::from("dev", "hntui", "hntui")
                .context("resolve OS cache dir")?;
            proj.cache_dir().to_path_buf()
        }
    };
    // `--no-file-cache` governs the HN item cache only.
    let cache_dir = (!cli.no_file_cache).then(|| resolved_cache_dir.clone());
    let state_store = cache_dir.clone().map(StateStore::new);
    let disk_cache = cache_dir.clone().map(|dir| DiskCacheConfig {
        dir,
        ttl: Duration::from_secs(cli.file_cache_ttl_secs.get()),
    });

    let backend = cli.api_backend;
    let base_url = cli.resolved_base_url();
    let http = reqwest::Client::builder()
        .pool_max_idle_per_host(10)
        .pool_idle_timeout(Duration::from_secs(30))
        .build()
        .context("build shared HTTP client")?;
    let client = HnClient::new(
        http.clone(),
        base_url,
        backend,
        cli.cache_size.get(),
        cli.concurrency.get(),
        disk_cache,
    )?;
    client.cleanup_disk_cache_background(Duration::from_secs(60 * 60 * 24));
    let search = SearchClient::new(http.clone(), "https://hn.algolia.com/api/v1/search")?;
    let summarizer = Summarizer::new(config.summarize().cloned(), config.api_key_override(), http);
    // localwebrs writes a CWD-relative `cache/cache.sqlite`, so it must run
    // from our cache dir or it litters the user's working directory. The dir
    // has to exist up front: a missing `current_dir` fails the spawn with the
    // same ENOENT a missing binary would, which would misreport as "install
    // localwebrs". Independent of --no-file-cache, which is about HN items.
    let article_dir = std::fs::create_dir_all(&resolved_cache_dir)
        .map(|()| resolved_cache_dir.clone())
        .map_err(|error| {
            logging::log_error(format!(
                "article cache dir unavailable {}: {error}",
                resolved_cache_dir.display()
            ));
        })
        .ok();
    let article_fetcher = ArticleFetcher::new(config.article_bin(), article_dir);
    let sources = Sources::new(Arc::new(client), Arc::new(search));

    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    let mut app = App::new(
        cli,
        sources,
        tx.clone(),
        state_store.clone(),
        config,
        summarizer,
        article_fetcher,
    );

    if let Some(store) = &state_store {
        if let Some(state) = store.load_story_list_state().await? {
            let feed = state.feed.as_deref().and_then(FeedKind::from_str_opt);
            app.seen_story_ids.extend(state.seen_story_ids);
            app.restore_story_list_state(state.story_ids, state.stories, feed);
        }
    }
    app.maybe_prefetch_comments();
    app.refresh_stories();

    let mut tui = Tui::init()?;
    let mut events = EventStream::new();

    loop {
        let area = tui.area()?;
        app.prepare_frame(area);
        if app.view == View::Stories {
            app.maybe_prefetch_stories();
        }
        tui.draw(|f| ui::render(f, &app))?;

        let tick_duration = if app.is_busy() {
            Duration::from_millis(120)
        } else {
            Duration::from_millis(200)
        };

        tokio::select! {
            maybe_event = events.next() => {
                let Some(event) = maybe_event else {
                    return Err(anyhow::anyhow!("crossterm event stream ended unexpectedly"));
                };

                let event = event.context("read terminal event")?;
                match event {
                    Event::Key(key) => app.handle_key(key),
                    Event::Mouse(mouse) => app.handle_mouse(mouse),
                    Event::Resize(_, _) => {}
                    _ => {}
                }
            }
            maybe_app_event = rx.recv() => {
                let Some(app_event) = maybe_app_event else {
                    return Err(anyhow::anyhow!("app event channel closed unexpectedly"));
                };
                app.handle_app_event(app_event);
            }
            _ = tokio::time::sleep(tick_duration) => {
                app.tick();
            }
        }

        if app.should_quit() {
            break;
        }
    }

    drop(tui);
    app.tasks.cancel_and_wait(TaskTarget::StoryStateSave).await;
    if let Some(store) = &state_store {
        if !app.story_ids.is_empty() && !app.stories.is_empty() {
            store
                .save_story_list_state(
                    app.story_ids.clone(),
                    app.stories.clone(),
                    app.current_feed.as_str().to_string(),
                    app.seen_story_ids.iter().copied().collect(),
                )
                .await?;
        }
    }

    Ok(())
}
