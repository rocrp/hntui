use crate::api::{CommentNode, DiskCacheConfig, HnClient, Story};
use crate::input::{Action, KeyState};
use crate::state::StateStore;
use crate::tui::Tui;
use crate::ui;
use crate::ui::theme;
use crate::Cli;
use anyhow::{Context, Result};
use crossterm::event::{Event, EventStream, KeyEventKind};
use futures::StreamExt;
use ratatui::widgets::ListState;
use std::cmp;
use std::collections::HashMap;
use std::time::Duration;
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
    CommentsPrefetched {
        generation: u64,
        story_id: u64,
        comments: Vec<CommentNode>,
    },
    CommentChildrenLoaded {
        generation: u64,
        parent_id: u64,
        children: Vec<CommentNode>,
    },
    CommentChildrenError {
        generation: u64,
        parent_id: u64,
        message: String,
    },
    Error {
        generation: u64,
        message: String,
    },
    PrefetchError {
        generation: u64,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoriesLoadMode {
    Replace,
    Append,
}

#[derive(Debug)]
struct PrefetchedComments {
    story_id: u64,
    comments: Vec<CommentNode>,
}

pub struct App {
    pub view: View,
    pub help_visible: bool,
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
    state_store: Option<StateStore>,

    stories_generation: u64,
    comments_generation: u64,
    comments_prefetch_generation: u64,
    pub prefetch_in_flight: bool,
    pub comment_prefetch_in_flight: bool,
    comment_prefetch_story_id: Option<u64>,
    prefetched_comments: Option<PrefetchedComments>,
    awaiting_prefetch_story_id: Option<u64>,
    input: KeyState,
    should_quit: bool,
    spinner_idx: usize,

    pending_story_selection_id: Option<u64>,

    comment_children_generation: u64,
    comment_children_in_flight: HashMap<u64, u64>,
}

impl App {
    pub fn new(
        cli: Cli,
        client: HnClient,
        tx: mpsc::UnboundedSender<AppEvent>,
        state_store: Option<StateStore>,
    ) -> Self {
        let mut story_list_state = ListState::default();
        story_list_state.select(Some(0));

        let mut comment_list_state = ListState::default();
        comment_list_state.select(Some(0));

        Self {
            view: View::Stories,
            help_visible: false,
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
            state_store,

            stories_generation: 0,
            comments_generation: 0,
            comments_prefetch_generation: 0,
            prefetch_in_flight: false,
            comment_prefetch_in_flight: false,
            comment_prefetch_story_id: None,
            prefetched_comments: None,
            awaiting_prefetch_story_id: None,
            input: KeyState::default(),
            should_quit: false,
            spinner_idx: 0,

            pending_story_selection_id: None,

            comment_children_generation: 0,
            comment_children_in_flight: HashMap::new(),
        }
    }

    pub fn spinner_frame(&self) -> char {
        const FRAMES: [char; 8] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧'];
        FRAMES[self.spinner_idx % FRAMES.len()]
    }

    pub fn tick(&mut self) {
        if self.is_busy() {
            self.spinner_idx = self.spinner_idx.wrapping_add(1);
        }
    }

    fn is_busy(&self) -> bool {
        self.story_loading
            || self.prefetch_in_flight
            || self.comment_loading
            || self.comment_prefetch_in_flight
            || !self.comment_children_in_flight.is_empty()
    }

    pub fn restore_story_list_state(&mut self, story_ids: Vec<u64>, stories: Vec<Story>) {
        if story_ids.is_empty() || stories.is_empty() {
            self.last_error = Some("refusing to restore empty story list state".to_string());
            return;
        }

        self.story_ids = story_ids;
        self.stories = stories;
        self.story_loading = false;
        self.prefetch_in_flight = false;
        self.story_list_state.select(Some(0));
        *self.story_list_state.offset_mut() = 0;
    }

    fn save_story_list_state_background(&self) {
        let Some(store) = self.state_store.clone() else {
            return;
        };
        if self.story_ids.is_empty() || self.stories.is_empty() {
            return;
        }

        let story_ids = self.story_ids.clone();
        let stories = self.stories.clone();
        tokio::spawn(async move {
            if let Err(err) = store.save_story_list_state(story_ids, stories).await {
                eprintln!("hntui: failed to save story list state: {err:#}");
            }
        });
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn refresh_stories(&mut self) {
        self.stories_generation = self.stories_generation.wrapping_add(1);
        let generation = self.stories_generation;

        self.pending_story_selection_id = self.selected_story().map(|s| s.id);

        self.last_error = None;
        self.story_loading = true;
        self.prefetch_in_flight = false;
        if self.stories.is_empty() {
            self.story_list_state.select(Some(0));
            *self.story_list_state.offset_mut() = 0;
        }

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
        let should_fill_viewport = loaded < self.story_page_size;
        let should_prefetch =
            should_fill_viewport || selected.saturating_mul(10) >= loaded.saturating_mul(8);
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

    pub fn maybe_prefetch_comments(&mut self) {
        if self.view != View::Stories {
            return;
        }
        if self.comment_prefetch_in_flight {
            return;
        }
        if self.story_loading && self.stories.is_empty() {
            return;
        }
        let Some(story) = self.selected_story().cloned() else {
            return;
        };
        if story.kids.is_empty() {
            return;
        }
        if self
            .prefetched_comments
            .as_ref()
            .is_some_and(|p| p.story_id == story.id)
        {
            return;
        }
        if self
            .comment_prefetch_story_id
            .is_some_and(|id| id == story.id)
        {
            return;
        }

        self.comments_prefetch_generation = self.comments_prefetch_generation.wrapping_add(1);
        let generation = self.comments_prefetch_generation;

        self.comment_prefetch_in_flight = true;
        self.comment_prefetch_story_id = Some(story.id);

        let story_id = story.id;
        let client = self.client.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let res = client.fetch_comment_roots(&story).await;
            match res {
                Ok(comments) => {
                    let _ = tx.send(AppEvent::CommentsPrefetched {
                        generation,
                        story_id,
                        comments,
                    });
                }
                Err(err) => {
                    let _ = tx.send(AppEvent::PrefetchError {
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

        if self
            .prefetched_comments
            .as_ref()
            .is_some_and(|p| p.story_id == story.id)
        {
            let prefetched = self
                .prefetched_comments
                .take()
                .expect("prefetched comments present");
            self.apply_comments_for_story(story, prefetched.comments, true);
            return;
        }

        if self
            .comment_prefetch_story_id
            .is_some_and(|id| id == story.id)
        {
            self.awaiting_prefetch_story_id = Some(story.id);
            self.view = View::Comments;
            self.last_error = None;
            self.current_story = Some(story);
            self.comment_loading = true;
            self.comment_tree.clear();
            self.comment_children_in_flight.clear();
            self.comment_list.clear();
            self.comment_list_state.select(Some(0));
            *self.comment_list_state.offset_mut() = 0;
            return;
        }

        self.load_comments_for_story(story, true);
    }

    fn load_comments_for_story(&mut self, story: Story, switch_view: bool) {
        self.comments_generation = self.comments_generation.wrapping_add(1);
        let generation = self.comments_generation;
        self.awaiting_prefetch_story_id = None;

        if switch_view {
            self.view = View::Comments;
        }

        self.last_error = None;
        self.current_story = Some(story.clone());
        self.comment_loading = true;
        self.comment_tree.clear();
        self.comment_children_in_flight.clear();
        self.comment_list.clear();
        self.comment_list_state.select(Some(0));
        *self.comment_list_state.offset_mut() = 0;

        let story_id = story.id;
        let client = self.client.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let res = client.fetch_comment_roots(&story).await;
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

    fn apply_comments_for_story(
        &mut self,
        story: Story,
        comments: Vec<CommentNode>,
        switch_view: bool,
    ) {
        if switch_view {
            self.view = View::Comments;
        }
        self.awaiting_prefetch_story_id = None;
        self.comment_loading = false;
        self.comment_children_in_flight.clear();
        self.last_error = None;
        self.current_story = Some(story);
        self.comment_tree = comments;
        self.apply_default_comment_expansion();
        self.rebuild_comment_list(None);
        self.comment_list_state.select(Some(0));
        *self.comment_list_state.offset_mut() = 0;
    }

    pub fn handle_action(&mut self, action: Action) {
        if action == Action::ToggleHelp {
            self.help_visible = !self.help_visible;
            return;
        }
        if self.help_visible {
            if action == Action::BackOrQuit {
                self.help_visible = false;
            }
            return;
        }

        match (self.view, action) {
            (View::Stories, Action::BackOrQuit) => self.should_quit = true,
            (View::Comments, Action::BackOrQuit) => {
                self.view = View::Stories;
                self.maybe_prefetch_comments();
            }
            (View::Stories, Action::Refresh) => self.refresh_stories(),
            (View::Comments, Action::Refresh) => self.refresh_comments(),

            (View::Stories, Action::Enter) => self.open_comments_for_selected_story(),
            (View::Stories, Action::OpenComments) => self.open_comments_for_selected_story(),
            (View::Stories, Action::Expand) => self.open_comments_for_selected_story(),
            (View::Stories, Action::OpenInBrowser) => {
                if let Err(err) = self.open_selected_story_in_browser() {
                    self.last_error = Some(format!("{err:#}"));
                }
            }
            (View::Comments, Action::OpenInBrowser) => {
                if let Err(err) = self.open_current_story_comments_in_browser() {
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
                self.maybe_prefetch_comments();
            }
            (View::Stories, Action::MoveUp) => {
                move_selection_up(&mut self.story_list_state);
                ensure_visible(
                    &mut self.story_list_state,
                    self.stories.len(),
                    self.story_page_size,
                );
                self.maybe_prefetch_comments();
            }
            (View::Stories, Action::PageDown) => {
                page_down(
                    &mut self.story_list_state,
                    self.stories.len(),
                    self.story_page_size,
                );
                self.maybe_prefetch_stories();
                self.maybe_prefetch_comments();
            }
            (View::Stories, Action::PageUp) => {
                page_up(&mut self.story_list_state, self.story_page_size);
                self.maybe_prefetch_comments();
            }
            (View::Stories, Action::GoTop) => {
                self.story_list_state.select(Some(0));
                *self.story_list_state.offset_mut() = 0;
                self.maybe_prefetch_comments();
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
                    self.maybe_prefetch_comments();
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
            (View::Comments, Action::Enter) => self.toggle_selected_comment_collapse(),
            (View::Comments, Action::Collapse) => self.collapse_selected_comment(),
            (View::Comments, Action::Expand) => self.expand_selected_comment(),
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
                        let select_idx = self
                            .pending_story_selection_id
                            .take()
                            .and_then(|id| self.stories.iter().position(|s| s.id == id))
                            .unwrap_or(0);
                        self.story_list_state.select(Some(select_idx));
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
                self.save_story_list_state_background();
                self.maybe_prefetch_comments();
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

                let story = self
                    .current_story
                    .clone()
                    .expect("current_story present for CommentsLoaded");
                self.apply_comments_for_story(story, comments, false);
            }
            AppEvent::CommentsPrefetched {
                generation,
                story_id,
                comments,
            } => {
                if generation != self.comments_prefetch_generation {
                    return;
                }

                self.comment_prefetch_in_flight = false;
                self.comment_prefetch_story_id = None;

                if self
                    .awaiting_prefetch_story_id
                    .is_some_and(|id| id == story_id)
                {
                    let story = self
                        .current_story
                        .clone()
                        .expect("current_story present when awaiting prefetch");
                    self.apply_comments_for_story(story, comments, false);
                    return;
                }

                self.prefetched_comments = Some(PrefetchedComments { story_id, comments });
                self.maybe_prefetch_comments();
            }
            AppEvent::CommentChildrenLoaded {
                generation,
                parent_id,
                children,
            } => {
                if self
                    .comment_children_in_flight
                    .get(&parent_id)
                    .copied()
                    .is_some_and(|g| g != generation)
                {
                    return;
                }
                if self.comment_children_in_flight.remove(&parent_id).is_none() {
                    return;
                }

                if attach_children_in_tree(&mut self.comment_tree, parent_id, children).is_none() {
                    self.last_error = Some(format!("comment not found id={parent_id}"));
                    return;
                }

                self.rebuild_comment_list(Some(parent_id));
                ensure_visible(
                    &mut self.comment_list_state,
                    self.comment_list.len(),
                    self.comment_page_size,
                );
            }
            AppEvent::CommentChildrenError {
                generation,
                parent_id,
                message,
            } => {
                if self
                    .comment_children_in_flight
                    .get(&parent_id)
                    .copied()
                    .is_some_and(|g| g != generation)
                {
                    return;
                }
                if self.comment_children_in_flight.remove(&parent_id).is_none() {
                    return;
                }
                let _ = set_children_loading_in_tree(&mut self.comment_tree, parent_id, false);
                let _ = set_collapse_in_tree(&mut self.comment_tree, parent_id, true);
                self.last_error = Some(message);
                self.rebuild_comment_list(Some(parent_id));
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
            AppEvent::PrefetchError {
                generation,
                message,
            } => {
                if generation != self.comments_prefetch_generation {
                    return;
                }
                self.comment_prefetch_in_flight = false;
                self.comment_prefetch_story_id = None;
                if self.awaiting_prefetch_story_id.is_some() {
                    self.awaiting_prefetch_story_id = None;
                    self.comment_loading = false;
                }
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

    fn open_current_story_comments_in_browser(&self) -> Result<()> {
        let story = self.current_story.as_ref().context("no current story")?;
        open_story_comments(story)
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

    fn apply_default_comment_expansion(&mut self) {
        let visible_levels = theme::layout().comment_default_visible_levels;
        let expand_depth_exclusive = visible_levels.saturating_sub(1);

        fn walk(nodes: &mut [CommentNode], expand_depth_exclusive: usize) {
            for node in nodes {
                if node.comment.depth < expand_depth_exclusive && !node.comment.kids.is_empty() {
                    node.comment.collapsed = false;
                }
                if !node.children.is_empty() {
                    walk(&mut node.children, expand_depth_exclusive);
                }
            }
        }

        walk(&mut self.comment_tree, expand_depth_exclusive);
    }

    fn start_loading_comment_children(&mut self, parent_id: u64) {
        if self.comment_children_in_flight.contains_key(&parent_id) {
            return;
        }

        let Some(info) = comment_info_in_tree(&self.comment_tree, parent_id) else {
            self.last_error = Some(format!("comment not found id={parent_id}"));
            return;
        };
        let (parent_depth, kids, children_loaded, children_loading) = info;

        if kids.is_empty() || children_loaded || children_loading {
            return;
        }

        self.comment_children_generation = self.comment_children_generation.wrapping_add(1);
        let generation = self.comment_children_generation;
        self.comment_children_in_flight
            .insert(parent_id, generation);

        if set_children_loading_in_tree(&mut self.comment_tree, parent_id, true).is_none() {
            self.last_error = Some(format!("comment not found id={parent_id}"));
            return;
        }
        if set_collapse_in_tree(&mut self.comment_tree, parent_id, false).is_none() {
            self.last_error = Some(format!("comment not found id={parent_id}"));
            return;
        }

        self.rebuild_comment_list(Some(parent_id));
        ensure_visible(
            &mut self.comment_list_state,
            self.comment_list.len(),
            self.comment_page_size,
        );

        let depth = parent_depth.saturating_add(1);
        let client = self.client.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let res = client.fetch_comment_children(&kids, depth).await;
            match res {
                Ok(children) => {
                    let _ = tx.send(AppEvent::CommentChildrenLoaded {
                        generation,
                        parent_id,
                        children,
                    });
                }
                Err(err) => {
                    let _ = tx.send(AppEvent::CommentChildrenError {
                        generation,
                        parent_id,
                        message: format!("{err:#}"),
                    });
                }
            }
        });
    }

    fn collapse_selected_comment(&mut self) {
        let Some(selected) = self.comment_list_state.selected() else {
            return;
        };
        let Some(comment) = self.comment_list.get(selected) else {
            return;
        };
        if comment.kids.is_empty() || comment.collapsed {
            return;
        }

        let id = comment.id;
        if set_collapse_in_tree(&mut self.comment_tree, id, true).is_none() {
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

    fn expand_selected_comment(&mut self) {
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
        let needs_load = !comment.children_loaded && !comment.children_loading;
        if set_collapse_in_tree(&mut self.comment_tree, id, false).is_none() {
            self.last_error = Some(format!("comment not found id={id}"));
            return;
        }

        if needs_load {
            self.start_loading_comment_children(id);
            return;
        }

        self.rebuild_comment_list(Some(id));
        ensure_visible(
            &mut self.comment_list_state,
            self.comment_list.len(),
            self.comment_page_size,
        );
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
        if comment.collapsed {
            self.expand_selected_comment();
        } else {
            self.collapse_selected_comment();
        }
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

fn open_story_comments(story: &Story) -> Result<()> {
    let url = format!("https://news.ycombinator.com/item?id={}", story.id);
    open::that(url).context("open comments in browser")?;
    Ok(())
}

fn set_collapse_in_tree(tree: &mut [CommentNode], target: u64, collapsed: bool) -> Option<()> {
    for node in tree {
        if node.comment.id == target {
            node.comment.collapsed = collapsed;
            return Some(());
        }
        if set_collapse_in_tree(&mut node.children, target, collapsed).is_some() {
            return Some(());
        }
    }
    None
}

fn comment_info_in_tree(
    tree: &[CommentNode],
    target: u64,
) -> Option<(usize, Vec<u64>, bool, bool)> {
    for node in tree {
        if node.comment.id == target {
            return Some((
                node.comment.depth,
                node.comment.kids.clone(),
                node.comment.children_loaded,
                node.comment.children_loading,
            ));
        }
        if let Some(found) = comment_info_in_tree(&node.children, target) {
            return Some(found);
        }
    }
    None
}

fn set_children_loading_in_tree(
    tree: &mut [CommentNode],
    target: u64,
    loading: bool,
) -> Option<()> {
    for node in tree {
        if node.comment.id == target {
            node.comment.children_loading = loading;
            return Some(());
        }
        if set_children_loading_in_tree(&mut node.children, target, loading).is_some() {
            return Some(());
        }
    }
    None
}

fn attach_children_in_tree(
    tree: &mut [CommentNode],
    target: u64,
    children: Vec<CommentNode>,
) -> Option<()> {
    fn inner(
        tree: &mut [CommentNode],
        target: u64,
        children: &mut Option<Vec<CommentNode>>,
    ) -> bool {
        for node in tree {
            if node.comment.id == target {
                node.children = children.take().expect("children not yet taken");
                node.comment.children_loaded = true;
                node.comment.children_loading = false;
                return true;
            }
            if inner(&mut node.children, target, children) {
                return true;
            }
        }
        false
    }

    let mut children = Some(children);
    if inner(tree, target, &mut children) {
        return Some(());
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
    let cache_dir = if cli.no_file_cache {
        None
    } else {
        Some(match cli.file_cache_dir.clone() {
            Some(dir) => dir,
            None => {
                let proj = directories::ProjectDirs::from("dev", "hntui", "hntui")
                    .context("resolve OS cache dir")?;
                proj.cache_dir().to_path_buf()
            }
        })
    };
    let state_store = cache_dir.clone().map(StateStore::new);
    let disk_cache = cache_dir.clone().map(|dir| DiskCacheConfig {
        dir,
        ttl: Duration::from_secs(cli.file_cache_ttl_secs),
    });

    let client = HnClient::new(
        cli.base_url.clone(),
        cli.cache_size,
        cli.concurrency,
        disk_cache,
    )?;

    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    let mut app = App::new(cli, client, tx.clone(), state_store.clone());

    if let Some(store) = &state_store {
        if let Some(state) = store.load_story_list_state().await? {
            app.restore_story_list_state(state.story_ids, state.stories);
        }
    }
    app.maybe_prefetch_comments();
    app.refresh_stories();

    let mut tui = Tui::init()?;
    let mut events = EventStream::new();

    loop {
        tui.draw(|f| ui::render(f, &mut app))?;

        let tick_duration = if app.is_busy() {
            Duration::from_millis(120)
        } else {
            Duration::from_secs(3600)
        };

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
            _ = tokio::time::sleep(tick_duration) => {
                app.tick();
            }
        }

        if app.should_quit() {
            break;
        }
    }

    drop(tui);
    if let Some(store) = &state_store {
        if !app.story_ids.is_empty() && !app.stories.is_empty() {
            store
                .save_story_list_state(app.story_ids.clone(), app.stories.clone())
                .await?;
        }
    }

    Ok(())
}
