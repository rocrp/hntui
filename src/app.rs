use crate::api::{CommentNode, HnClient, Story};
use crate::input::{Action, KeyState};
use crate::tui::Tui;
use crate::ui;
use crate::Cli;
use anyhow::{Context, Result};
use crossterm::event::{Event, EventStream, KeyEventKind};
use futures::StreamExt;
use ratatui::widgets::ListState;
use std::cmp;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Stories,
    Comments,
}

#[derive(Debug)]
pub enum AppEvent {
    StoriesLoaded {
        generation: u64,
        mode: StoriesLoadMode,
        story_ids: Option<Vec<u64>>,
        stories: Vec<Story>,
    },
    CommentsLoaded {
        generation: u64,
        story_id: u64,
        comments: Vec<CommentNode>,
    },
    Error {
        generation: u64,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoriesLoadMode {
    Replace,
    Append,
}

pub struct App {
    pub view: View,
    pub stories: Vec<Story>,
    pub story_ids: Vec<u64>,
    pub story_list_state: ListState,
    pub story_loading: bool,
    pub story_page_size: usize,

    pub current_story: Option<Story>,
    pub comment_tree: Vec<CommentNode>,
    pub comment_list: Vec<crate::api::types::Comment>,
    pub comment_list_state: ListState,
    pub comment_loading: bool,
    pub comment_page_size: usize,

    pub last_error: Option<String>,

    client: HnClient,
    cli: Cli,
    tx: mpsc::UnboundedSender<AppEvent>,

    stories_generation: u64,
    comments_generation: u64,
    pub prefetch_in_flight: bool,
    input: KeyState,
    should_quit: bool,
}

impl App {
    pub fn new(cli: Cli, client: HnClient, tx: mpsc::UnboundedSender<AppEvent>) -> Self {
        let mut story_list_state = ListState::default();
        story_list_state.select(Some(0));

        let mut comment_list_state = ListState::default();
        comment_list_state.select(Some(0));

        Self {
            view: View::Stories,
            stories: vec![],
            story_ids: vec![],
            story_list_state,
            story_loading: false,
            story_page_size: 10,

            current_story: None,
            comment_tree: vec![],
            comment_list: vec![],
            comment_list_state,
            comment_loading: false,
            comment_page_size: 10,

            last_error: None,

            client,
            cli,
            tx,

            stories_generation: 0,
            comments_generation: 0,
            prefetch_in_flight: false,
            input: KeyState::default(),
            should_quit: false,
        }
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn refresh_stories(&mut self) {
        self.stories_generation = self.stories_generation.wrapping_add(1);
        let generation = self.stories_generation;

        self.last_error = None;
        self.story_loading = true;
        self.prefetch_in_flight = false;
        self.story_ids.clear();
        self.stories.clear();
        self.story_list_state.select(Some(0));
        *self.story_list_state.offset_mut() = 0;

        let client = self.client.clone();
        let tx = self.tx.clone();
        let count = self.cli.count;
        tokio::spawn(async move {
            let res = async {
                let story_ids = client.fetch_top_story_ids().await?;
                let ids = story_ids.iter().copied().take(count).collect::<Vec<_>>();
                let stories = client.fetch_stories_batch(&ids).await?;
                Ok::<_, anyhow::Error>((story_ids, stories))
            }
            .await;

            match res {
                Ok((story_ids, stories)) => {
                    let _ = tx.send(AppEvent::StoriesLoaded {
                        generation,
                        mode: StoriesLoadMode::Replace,
                        story_ids: Some(story_ids),
                        stories,
                    });
                }
                Err(err) => {
                    let _ = tx.send(AppEvent::Error {
                        generation,
                        message: format!("{err:#}"),
                    });
                }
            }
        });
    }

    pub fn refresh_comments(&mut self) {
        let Some(story) = self.current_story.clone() else {
            self.last_error = Some("no current story".to_string());
            return;
        };
        self.load_comments_for_story(story, true);
    }

    pub fn maybe_prefetch_stories(&mut self) {
        if self.story_loading || self.prefetch_in_flight {
            return;
        }
        if self.story_ids.is_empty() || self.stories.is_empty() {
            return;
        }

        let selected = self.story_list_state.selected().unwrap_or(0);
        let loaded = self.stories.len();
        let should_prefetch = selected.saturating_mul(10) >= loaded.saturating_mul(8);
        if !should_prefetch {
            return;
        }

        let start = loaded;
        if start >= self.story_ids.len() {
            return;
        }

        let end = cmp::min(start + self.cli.page_size, self.story_ids.len());
        let ids = self.story_ids[start..end].to_vec();

        self.prefetch_in_flight = true;
        let generation = self.stories_generation;
        let client = self.client.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let res = client.fetch_stories_batch(&ids).await;
            match res {
                Ok(stories) => {
                    let _ = tx.send(AppEvent::StoriesLoaded {
                        generation,
                        mode: StoriesLoadMode::Append,
                        story_ids: None,
                        stories,
                    });
                }
                Err(err) => {
                    let _ = tx.send(AppEvent::Error {
                        generation,
                        message: format!("{err:#}"),
                    });
                }
            }
        });
    }

    pub fn open_comments_for_selected_story(&mut self) {
        let Some(story) = self.selected_story().cloned() else {
            return;
        };

        if self
            .current_story
            .as_ref()
            .is_some_and(|s| s.id == story.id)
            && !self.comment_tree.is_empty()
            && !self.comment_loading
        {
            self.view = View::Comments;
            return;
        }

        self.load_comments_for_story(story, true);
    }

    fn load_comments_for_story(&mut self, story: Story, switch_view: bool) {
        self.comments_generation = self.comments_generation.wrapping_add(1);
        let generation = self.comments_generation;

        if switch_view {
            self.view = View::Comments;
        }

        self.last_error = None;
        self.current_story = Some(story.clone());
        self.comment_loading = true;
        self.comment_tree.clear();
        self.comment_list.clear();
        self.comment_list_state.select(Some(0));
        *self.comment_list_state.offset_mut() = 0;

        let story_id = story.id;
        let client = self.client.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let res = client.fetch_comments(&story).await;
            match res {
                Ok(comments) => {
                    let _ = tx.send(AppEvent::CommentsLoaded {
                        generation,
                        story_id,
                        comments,
                    });
                }
                Err(err) => {
                    let _ = tx.send(AppEvent::Error {
                        generation,
                        message: format!("{err:#}"),
                    });
                }
            }
        });
    }

    pub fn handle_action(&mut self, action: Action) {
        match (self.view, action) {
            (View::Stories, Action::BackOrQuit) => self.should_quit = true,
            (View::Comments, Action::BackOrQuit) => self.view = View::Stories,
            (View::Stories, Action::Refresh) => self.refresh_stories(),
            (View::Comments, Action::Refresh) => self.refresh_comments(),

            (View::Stories, Action::OpenComments) => self.open_comments_for_selected_story(),
            (View::Stories, Action::OpenInBrowser) => {
                if let Err(err) = self.open_selected_story_in_browser() {
                    self.last_error = Some(format!("{err:#}"));
                }
            }
            (View::Comments, Action::OpenInBrowser) => {
                if let Err(err) = self.open_current_story_in_browser() {
                    self.last_error = Some(format!("{err:#}"));
                }
            }

            (View::Stories, Action::MoveDown) => {
                move_selection_down(&mut self.story_list_state, self.stories.len());
                ensure_visible(
                    &mut self.story_list_state,
                    self.stories.len(),
                    self.story_page_size,
                );
                self.maybe_prefetch_stories();
            }
            (View::Stories, Action::MoveUp) => {
                move_selection_up(&mut self.story_list_state);
                ensure_visible(
                    &mut self.story_list_state,
                    self.stories.len(),
                    self.story_page_size,
                );
            }
            (View::Stories, Action::PageDown) => {
                page_down(
                    &mut self.story_list_state,
                    self.stories.len(),
                    self.story_page_size,
                );
                self.maybe_prefetch_stories();
            }
            (View::Stories, Action::PageUp) => {
                page_up(&mut self.story_list_state, self.story_page_size);
            }
            (View::Stories, Action::GoTop) => {
                self.story_list_state.select(Some(0));
                *self.story_list_state.offset_mut() = 0;
            }
            (View::Stories, Action::GoBottom) => {
                if !self.stories.is_empty() {
                    self.story_list_state.select(Some(self.stories.len() - 1));
                    ensure_visible(
                        &mut self.story_list_state,
                        self.stories.len(),
                        self.story_page_size,
                    );
                    self.maybe_prefetch_stories();
                }
            }

            (View::Comments, Action::MoveDown) => {
                move_selection_down(&mut self.comment_list_state, self.comment_list.len());
                ensure_visible(
                    &mut self.comment_list_state,
                    self.comment_list.len(),
                    self.comment_page_size,
                );
            }
            (View::Comments, Action::MoveUp) => {
                move_selection_up(&mut self.comment_list_state);
                ensure_visible(
                    &mut self.comment_list_state,
                    self.comment_list.len(),
                    self.comment_page_size,
                );
            }
            (View::Comments, Action::PageDown) => {
                page_down(
                    &mut self.comment_list_state,
                    self.comment_list.len(),
                    self.comment_page_size,
                );
            }
            (View::Comments, Action::PageUp) => {
                page_up(&mut self.comment_list_state, self.comment_page_size);
            }
            (View::Comments, Action::GoTop) => {
                self.comment_list_state.select(Some(0));
                *self.comment_list_state.offset_mut() = 0;
            }
            (View::Comments, Action::GoBottom) => {
                if !self.comment_list.is_empty() {
                    self.comment_list_state
                        .select(Some(self.comment_list.len() - 1));
                    ensure_visible(
                        &mut self.comment_list_state,
                        self.comment_list.len(),
                        self.comment_page_size,
                    );
                }
            }
            (View::Comments, Action::ToggleCollapse) => self.toggle_selected_comment_collapse(),

            (_, _) => {}
        }
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        if let Some(action) = self.input.on_key(key) {
            self.handle_action(action);
        }
    }

    pub fn handle_app_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::StoriesLoaded {
                generation,
                mode,
                story_ids,
                stories,
            } => {
                if generation != self.stories_generation {
                    return;
                }
                self.story_loading = false;
                self.prefetch_in_flight = false;
                self.last_error = None;

                if let Some(story_ids) = story_ids {
                    self.story_ids = story_ids;
                }

                match mode {
                    StoriesLoadMode::Replace => {
                        self.stories = stories;
                        self.story_list_state.select(Some(0));
                        *self.story_list_state.offset_mut() = 0;
                    }
                    StoriesLoadMode::Append => {
                        self.stories.extend(stories);
                    }
                }

                ensure_visible(
                    &mut self.story_list_state,
                    self.stories.len(),
                    self.story_page_size,
                );
            }
            AppEvent::CommentsLoaded {
                generation,
                story_id,
                comments,
            } => {
                if generation != self.comments_generation {
                    return;
                }
                if self
                    .current_story
                    .as_ref()
                    .is_some_and(|s| s.id != story_id)
                {
                    return;
                }

                self.comment_loading = false;
                self.last_error = None;
                self.comment_tree = comments;
                self.rebuild_comment_list(None);
                self.comment_list_state.select(Some(0));
                *self.comment_list_state.offset_mut() = 0;
            }
            AppEvent::Error {
                generation,
                message,
            } => {
                if generation != self.stories_generation && generation != self.comments_generation {
                    return;
                }
                self.story_loading = false;
                self.prefetch_in_flight = false;
                self.comment_loading = false;
                self.last_error = Some(message);
            }
        }
    }

    pub fn selected_story(&self) -> Option<&Story> {
        let idx = self.story_list_state.selected().unwrap_or(0);
        self.stories.get(idx)
    }

    fn open_selected_story_in_browser(&self) -> Result<()> {
        let story = self.selected_story().context("no selected story")?;
        open_story(story)
    }

    fn open_current_story_in_browser(&self) -> Result<()> {
        let story = self.current_story.as_ref().context("no current story")?;
        open_story(story)
    }

    fn rebuild_comment_list(&mut self, preserve_comment_id: Option<u64>) {
        fn walk(nodes: &[CommentNode], out: &mut Vec<crate::api::types::Comment>) {
            for node in nodes {
                out.push(node.comment.clone());
                if !node.comment.collapsed {
                    walk(&node.children, out);
                }
            }
        }

        let mut flat = Vec::new();
        walk(&self.comment_tree, &mut flat);
        self.comment_list = flat;

        let Some(id) = preserve_comment_id else {
            return;
        };
        if let Some(idx) = self.comment_list.iter().position(|c| c.id == id) {
            self.comment_list_state.select(Some(idx));
        }
    }

    fn toggle_selected_comment_collapse(&mut self) {
        let Some(selected) = self.comment_list_state.selected() else {
            return;
        };
        let Some(comment) = self.comment_list.get(selected) else {
            return;
        };
        if comment.kids.is_empty() {
            return;
        }

        let id = comment.id;
        if toggle_collapse_in_tree(&mut self.comment_tree, id).is_none() {
            self.last_error = Some(format!("comment not found id={id}"));
            return;
        }

        self.rebuild_comment_list(Some(id));
        ensure_visible(
            &mut self.comment_list_state,
            self.comment_list.len(),
            self.comment_page_size,
        );
    }
}

fn open_story(story: &Story) -> Result<()> {
    let url = story
        .url
        .clone()
        .unwrap_or_else(|| format!("https://news.ycombinator.com/item?id={}", story.id));
    open::that(url).context("open in browser")?;
    Ok(())
}

fn toggle_collapse_in_tree(tree: &mut [CommentNode], target: u64) -> Option<()> {
    for node in tree {
        if node.comment.id == target {
            node.comment.collapsed = !node.comment.collapsed;
            return Some(());
        }
        if toggle_collapse_in_tree(&mut node.children, target).is_some() {
            return Some(());
        }
    }
    None
}

fn move_selection_down(state: &mut ListState, len: usize) {
    if len == 0 {
        state.select(None);
        *state.offset_mut() = 0;
        return;
    }
    let selected = state.selected().unwrap_or(0);
    let next = cmp::min(selected + 1, len - 1);
    state.select(Some(next));
}

fn move_selection_up(state: &mut ListState) {
    let Some(selected) = state.selected() else {
        return;
    };
    state.select(Some(selected.saturating_sub(1)));
}

fn page_down(state: &mut ListState, len: usize, page_size: usize) {
    if len == 0 {
        state.select(None);
        *state.offset_mut() = 0;
        return;
    }
    let selected = state.selected().unwrap_or(0);
    let page_size = cmp::max(page_size, 1);
    let next = cmp::min(selected + page_size, len - 1);
    state.select(Some(next));
    ensure_visible(state, len, page_size);
}

fn page_up(state: &mut ListState, page_size: usize) {
    let Some(selected) = state.selected() else {
        return;
    };
    let page_size = cmp::max(page_size, 1);
    state.select(Some(selected.saturating_sub(page_size)));
    ensure_visible(state, selected + 1, page_size);
}

fn ensure_visible(state: &mut ListState, len: usize, page_size: usize) {
    if len == 0 {
        *state.offset_mut() = 0;
        return;
    }
    let Some(selected) = state.selected() else {
        *state.offset_mut() = 0;
        return;
    };
    let page_size = cmp::max(page_size, 1);

    let offset = state.offset();
    if selected < offset {
        *state.offset_mut() = selected;
    } else if selected >= offset + page_size {
        *state.offset_mut() = selected.saturating_sub(page_size - 1);
    }
}

pub async fn run(cli: Cli) -> Result<()> {
    let client = HnClient::new(cli.base_url.clone(), cli.cache_size, cli.concurrency)?;

    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    let mut app = App::new(cli, client, tx.clone());
    app.refresh_stories();

    let mut tui = Tui::init()?;
    let mut events = EventStream::new();

    loop {
        tui.draw(|f| ui::render(f, &mut app))?;

        tokio::select! {
            maybe_event = events.next() => {
                let Some(event) = maybe_event else {
                    return Err(anyhow::anyhow!("crossterm event stream ended unexpectedly"));
                };

                let event = event.context("read terminal event")?;
                match event {
                    Event::Key(key) => app.handle_key(key),
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
        }

        if app.should_quit() {
            break;
        }
    }

    Ok(())
}
