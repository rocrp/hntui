use super::{App, AppEvent, TaskTarget, View};
use crate::api::{FeedKind, SearchClient, Sources};
use crate::config::Config;
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

/// Ceiling on how many queued events one frame absorbs, so draining a backlog
/// cannot starve keyboard input.
const MAX_EVENTS_PER_FRAME: usize = 256;

pub async fn run(cli: Cli, config: Config) -> Result<()> {
    // `--no-file-cache` governs the HN item cache and story list state only.
    let state_store = if cli.no_file_cache {
        None
    } else {
        Some(StateStore::new(cli.cache_dir()?))
    };
    let http = reqwest::Client::builder()
        .pool_max_idle_per_host(10)
        .pool_idle_timeout(Duration::from_secs(30))
        .build()
        .context("build shared HTTP client")?;
    let client = cli.hn_client(http.clone())?;
    client.cleanup_disk_cache_background(Duration::from_secs(60 * 60 * 24));
    let search = SearchClient::new(http.clone(), "https://hn.algolia.com/api/v1/search")?;
    let summarizer = Summarizer::new(
        config.summarize().cloned(),
        config.api_key_override(),
        http.clone(),
    );
    let article_fetcher = cli.article_fetcher(&config)?;
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
    )
    .with_paste_service(Arc::new(crate::handoff::JakePasteService::new(
        http.clone(),
    )));

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
                // One redraw per event is fine at streaming speed, but a
                // backlog is not: a summary that ran for the length of an
                // editing session leaves thousands of chunks queued, and
                // painting each one costs seconds of unresponsiveness. Take
                // what is already waiting, capped so a fast producer cannot
                // keep the loop from reading the keyboard.
                for _ in 0..MAX_EVENTS_PER_FRAME {
                    match rx.try_recv() {
                        Ok(app_event) => app.handle_app_event(app_event),
                        Err(_) => break,
                    }
                }
            }
            _ = tokio::time::sleep(tick_duration) => {
                app.tick();
            }
        }

        if app.take_editor_request() {
            events = hand_terminal_to_editor(&mut tui, events, &mut app)?;
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

/// Hands the terminal to the Editor and takes it back.
///
/// The event stream goes first and comes back last: its reader thread blocks on
/// the tty, so leaving it alive would let it eat the keystrokes meant for the
/// editor.
fn hand_terminal_to_editor(
    tui: &mut Tui,
    events: EventStream,
    app: &mut App,
) -> Result<EventStream> {
    let path = app.config_path_for_editing();

    // Taken by value so it is really gone before the editor starts, not merely
    // replaced. Dropping it signals crossterm's reader thread to stop rather
    // than joining it, so it may still be in a blocking read for a moment; a
    // keypress landing in that window is buffered in crossterm's global reader,
    // shared with the stream built below, and replayed after the handoff rather
    // than swallowed.
    drop(events);
    tui.suspend().context("hand the terminal to the editor")?;

    // Blocking, and on a single-core host that is the only worker thread: a
    // streaming summary or an article fetch would freeze for the whole editing
    // session and be dropped server-side.
    let outcome = tokio::task::block_in_place(|| crate::editor::edit(&path));

    tui.resume().context("take the terminal back")?;
    app.finish_config_edit(outcome);
    Ok(EventStream::new())
}
