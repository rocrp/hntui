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
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
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
        story_id: u64,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoriesLoadMode {
    Replace,
    Append,
}

const IDLE_PREFETCH_DELAY: Duration = Duration::from_millis(500);
const MAX_COMMENT_PREFETCH_IN_FLIGHT: usize = 3;

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
    pub comment_item_heights: Vec<usize>,
    pub comment_viewport_height: usize,
    pub comment_line_offset: usize,

    pub last_error: Option<String>,

    client: HnClient,
    cli: Cli,
    tx: mpsc::UnboundedSender<AppEvent>,
    state_store: Option<StateStore>,

    stories_generation: u64,
    comments_generation: u64,
    comments_prefetch_generation: u64,
    pub prefetch_in_flight: bool,
    pub comment_prefetch_in_flight_ids: HashSet<u64>,
    comment_prefetch_generations: HashMap<u64, u64>,
    prefetched_comments_cache: HashMap<u64, Vec<CommentNode>>,
    awaiting_prefetch_story_id: Option<u64>,
    input: KeyState,
    should_quit: bool,
    spinner_idx: usize,
    last_user_activity: Instant,

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
            comment_item_heights: Vec::new(),
            comment_viewport_height: 0,
            comment_line_offset: 0,

            last_error: None,

            client,
            cli,
            tx,
            state_store,

            stories_generation: 0,
            comments_generation: 0,
            comments_prefetch_generation: 0,
            prefetch_in_flight: false,
            comment_prefetch_in_flight_ids: HashSet::new(),
            comment_prefetch_generations: HashMap::new(),
            prefetched_comments_cache: HashMap::new(),
            awaiting_prefetch_story_id: None,
            input: KeyState::default(),
            should_quit: false,
            spinner_idx: 0,
            last_user_activity: Instant::now(),

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
        self.maybe_prefetch_comments();
    }

    fn is_busy(&self) -> bool {
        self.story_loading
            || self.prefetch_in_flight
            || self.comment_loading
            || !self.comment_prefetch_in_flight_ids.is_empty()
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

    pub fn ensure_comment_line_offset(&mut self) {
        ensure_comment_line_offset(
            &mut self.comment_list_state,
            &mut self.comment_line_offset,
            &self.comment_item_heights,
            self.comment_viewport_height,
        );
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
                let story_ids = client.fetch_top_story_ids_force().await?;
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
        if self.comment_prefetch_in_flight_ids.len() >= MAX_COMMENT_PREFETCH_IN_FLIGHT {
            return;
        }
        if self.story_loading && self.stories.is_empty() {
            return;
        }
        if !self.is_idle_for_prefetch() {
            return;
        }

        let candidates = self.prefetch_story_candidates();
        if candidates.is_empty() {
            return;
        }

        for story in candidates {
            if self.comment_prefetch_in_flight_ids.len() >= MAX_COMMENT_PREFETCH_IN_FLIGHT {
                break;
            }
            self.start_comment_prefetch(story);
        }
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
        {
            self.view = View::Comments;
            return;
        }

        if let Some(comments) = self.prefetched_comments_cache.remove(&story.id) {
            self.apply_comments_for_story(story, comments, true);
            return;
        }

        if self.comment_prefetch_in_flight_ids.contains(&story.id) {
            self.awaiting_prefetch_story_id = Some(story.id);
            self.view = View::Comments;
            self.last_error = None;
            let is_same_story = self
                .current_story
                .as_ref()
                .is_some_and(|current| current.id == story.id);
            self.current_story = Some(story);
            self.comment_loading = true;
            if !is_same_story {
                self.reset_comment_state();
            }
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
        let is_same_story = self
            .current_story
            .as_ref()
            .is_some_and(|current| current.id == story.id);
        self.current_story = Some(story.clone());
        self.comment_loading = true;
        if !is_same_story {
            self.reset_comment_state();
        }

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
        self.comment_line_offset = 0;
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
            (View::Stories, Action::OpenPrimaryBrowser) => {
                if let Err(err) = self.open_selected_story_in_browser() {
                    self.last_error = Some(format!("{err:#}"));
                }
            }
            (View::Stories, Action::OpenSecondaryBrowser) => {
                if let Err(err) = self.open_selected_story_comments_in_browser() {
                    self.last_error = Some(format!("{err:#}"));
                }
            }
            (View::Comments, Action::OpenPrimaryBrowser) => {
                if let Err(err) = self.open_current_story_comments_in_browser() {
                    self.last_error = Some(format!("{err:#}"));
                }
            }
            (View::Comments, Action::OpenSecondaryBrowser) => {
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
                let comment_len = self.comment_list.len();
                move_selection_down(&mut self.comment_list_state, comment_len);
                ensure_comment_visible(
                    &mut self.comment_list_state,
                    &mut self.comment_line_offset,
                    comment_len,
                    &self.comment_item_heights,
                    self.comment_viewport_height,
                );
            }
            (View::Comments, Action::MoveUp) => {
                move_selection_up(&mut self.comment_list_state);
                ensure_comment_visible(
                    &mut self.comment_list_state,
                    &mut self.comment_line_offset,
                    self.comment_list.len(),
                    &self.comment_item_heights,
                    self.comment_viewport_height,
                );
            }
            (View::Comments, Action::PageDown) => {
                page_down_comment_list(
                    &mut self.comment_list_state,
                    self.comment_list.len(),
                    self.comment_page_size,
                    &mut self.comment_line_offset,
                    &self.comment_item_heights,
                    self.comment_viewport_height,
                );
            }
            (View::Comments, Action::PageUp) => {
                page_up_comment_list(
                    &mut self.comment_list_state,
                    self.comment_list.len(),
                    self.comment_page_size,
                    &mut self.comment_line_offset,
                    &self.comment_item_heights,
                    self.comment_viewport_height,
                );
            }
            (View::Comments, Action::GoTop) => {
                self.comment_list_state.select(Some(0));
                ensure_comment_visible(
                    &mut self.comment_list_state,
                    &mut self.comment_line_offset,
                    self.comment_list.len(),
                    &self.comment_item_heights,
                    self.comment_viewport_height,
                );
            }
            (View::Comments, Action::GoBottom) => {
                if !self.comment_list.is_empty() {
                    self.comment_list_state
                        .select(Some(self.comment_list.len() - 1));
                    ensure_comment_visible(
                        &mut self.comment_list_state,
                        &mut self.comment_line_offset,
                        self.comment_list.len(),
                        &self.comment_item_heights,
                        self.comment_viewport_height,
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
        self.last_user_activity = Instant::now();
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
                        self.prefetched_comments_cache.clear();
                        self.comment_prefetch_in_flight_ids.clear();
                        self.comment_prefetch_generations.clear();
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
                let expected = self.comment_prefetch_generations.get(&story_id).copied();
                if expected != Some(generation) {
                    return;
                }

                self.comment_prefetch_in_flight_ids.remove(&story_id);
                self.comment_prefetch_generations.remove(&story_id);

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

                self.prefetched_comments_cache.insert(story_id, comments);
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
                ensure_comment_visible(
                    &mut self.comment_list_state,
                    &mut self.comment_line_offset,
                    self.comment_list.len(),
                    &self.comment_item_heights,
                    self.comment_viewport_height,
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
                story_id,
                message,
            } => {
                let expected = self.comment_prefetch_generations.get(&story_id).copied();
                if expected != Some(generation) {
                    return;
                }
                self.comment_prefetch_in_flight_ids.remove(&story_id);
                self.comment_prefetch_generations.remove(&story_id);
                if self.awaiting_prefetch_story_id.is_some() {
                    self.awaiting_prefetch_story_id = None;
                    self.comment_loading = false;
                }
                self.last_error = Some(message);
                self.maybe_prefetch_comments();
            }
        }
    }

    pub fn selected_story(&self) -> Option<&Story> {
        let idx = self.story_list_state.selected().unwrap_or(0);
        self.stories.get(idx)
    }

    pub fn is_comment_prefetching_for_story(&self, story_id: u64) -> bool {
        self.comment_prefetch_in_flight_ids.contains(&story_id)
    }

    fn reset_comment_state(&mut self) {
        self.comment_tree.clear();
        self.comment_children_in_flight.clear();
        self.comment_list.clear();
        self.comment_item_heights.clear();
        self.comment_line_offset = 0;
        self.comment_list_state.select(Some(0));
        *self.comment_list_state.offset_mut() = 0;
    }

    fn is_idle_for_prefetch(&self) -> bool {
        self.last_user_activity.elapsed() >= IDLE_PREFETCH_DELAY
    }

    fn prefetch_story_candidates(&self) -> Vec<Story> {
        let len = self.stories.len();
        if len == 0 {
            return Vec::new();
        }

        let offset = self.story_list_state.offset().min(len);
        let page_size = self.story_page_size.max(1);
        let end = (offset + page_size).min(len);
        let selected = self.story_list_state.selected().unwrap_or(offset);

        let mut indices = (offset..end).collect::<Vec<_>>();
        indices.sort_by_key(|idx| idx.abs_diff(selected));

        let mut out = Vec::new();
        for idx in indices {
            let Some(story) = self.stories.get(idx) else {
                continue;
            };
            if !self.can_prefetch_story(story) {
                continue;
            }
            out.push(story.clone());
        }

        out
    }

    fn can_prefetch_story(&self, story: &Story) -> bool {
        if story.kids.is_empty() {
            return false;
        }
        if self.prefetched_comments_cache.contains_key(&story.id) {
            return false;
        }
        if self.comment_prefetch_in_flight_ids.contains(&story.id) {
            return false;
        }
        true
    }

    fn start_comment_prefetch(&mut self, story: Story) {
        self.comments_prefetch_generation = self.comments_prefetch_generation.wrapping_add(1);
        let generation = self.comments_prefetch_generation;

        self.comment_prefetch_in_flight_ids.insert(story.id);
        self.comment_prefetch_generations
            .insert(story.id, generation);

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
                        story_id,
                        message: format!("{err:#}"),
                    });
                }
            }
        });
    }

    fn open_selected_story_in_browser(&self) -> Result<()> {
        let story = self.selected_story().context("no selected story")?;
        open_story(story)
    }

    fn open_selected_story_comments_in_browser(&self) -> Result<()> {
        let story = self.selected_story().context("no selected story")?;
        open_story_comments(story)
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
        self.comment_item_heights.clear();

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
        ensure_comment_visible(
            &mut self.comment_list_state,
            &mut self.comment_line_offset,
            self.comment_list.len(),
            &self.comment_item_heights,
            self.comment_viewport_height,
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
        ensure_comment_visible(
            &mut self.comment_list_state,
            &mut self.comment_line_offset,
            self.comment_list.len(),
            &self.comment_item_heights,
            self.comment_viewport_height,
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
        ensure_comment_visible(
            &mut self.comment_list_state,
            &mut self.comment_line_offset,
            self.comment_list.len(),
            &self.comment_item_heights,
            self.comment_viewport_height,
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

fn comment_heights_ready(len: usize, item_heights: &[usize], viewport_height: usize) -> bool {
    if len == 0 || viewport_height == 0 {
        return false;
    }
    if item_heights.is_empty() {
        return false;
    }
    if item_heights.len() != len {
        panic!(
            "comment item heights out of sync: expected {len}, got {}",
            item_heights.len()
        );
    }
    true
}

fn comment_total_lines(item_heights: &[usize]) -> usize {
    item_heights
        .iter()
        .map(|height| (*height).max(1))
        .sum()
}

fn comment_line_range(item_heights: &[usize], index: usize) -> (usize, usize) {
    let mut start = 0usize;
    for (idx, height) in item_heights.iter().enumerate() {
        let height = (*height).max(1);
        if idx == index {
            return (start, start + height);
        }
        start += height;
    }
    (start, start)
}

fn ensure_comment_line_offset(
    state: &mut ListState,
    line_offset: &mut usize,
    item_heights: &[usize],
    viewport_height: usize,
) {
    if item_heights.is_empty() || viewport_height == 0 {
        *line_offset = 0;
        return;
    }
    let Some(selected) = state.selected() else {
        *line_offset = 0;
        return;
    };

    let len = item_heights.len();
    let selected = if selected >= len {
        let last = len - 1;
        state.select(Some(last));
        last
    } else {
        selected
    };

    let viewport_height = viewport_height.max(1);
    let total_lines = comment_total_lines(item_heights);
    let max_offset = total_lines.saturating_sub(viewport_height);

    let (start, end) = comment_line_range(item_heights, selected);
    let height = end.saturating_sub(start);
    if height >= viewport_height {
        *line_offset = start.min(max_offset);
        return;
    }

    let mut offset = (*line_offset).min(max_offset);
    if start < offset {
        offset = start;
    } else if end > offset + viewport_height {
        offset = end.saturating_sub(viewport_height);
    }
    *line_offset = offset.min(max_offset);
}

fn page_down_with_heights(
    state: &mut ListState,
    item_heights: &[usize],
    viewport_height: usize,
) {
    let len = item_heights.len();
    if len == 0 {
        state.select(None);
        *state.offset_mut() = 0;
        return;
    }

    let selected = state.selected().unwrap_or(0).min(len - 1);
    let viewport_height = viewport_height.max(1);

    let mut target = selected;
    let mut used = 0usize;
    while target + 1 < len {
        let next = target + 1;
        let height = item_heights[next].max(1);
        if used == 0 && height >= viewport_height {
            target = next;
            break;
        }
        if used + height > viewport_height {
            break;
        }
        used += height;
        target = next;
    }

    state.select(Some(target));
}

fn page_up_with_heights(
    state: &mut ListState,
    item_heights: &[usize],
    viewport_height: usize,
) {
    let len = item_heights.len();
    if len == 0 {
        state.select(None);
        *state.offset_mut() = 0;
        return;
    }

    let selected = state.selected().unwrap_or(0).min(len - 1);
    let viewport_height = viewport_height.max(1);

    let mut target = selected;
    let mut used = 0usize;
    while target > 0 {
        let prev = target - 1;
        let height = item_heights[prev].max(1);
        if used == 0 && height >= viewport_height {
            target = prev;
            break;
        }
        if used + height > viewport_height {
            break;
        }
        used += height;
        target = prev;
    }

    state.select(Some(target));
}

fn ensure_comment_visible(
    state: &mut ListState,
    line_offset: &mut usize,
    len: usize,
    item_heights: &[usize],
    viewport_height: usize,
) {
    if comment_heights_ready(len, item_heights, viewport_height) {
        ensure_comment_line_offset(state, line_offset, item_heights, viewport_height);
    }
}

fn page_down_comment_list(
    state: &mut ListState,
    len: usize,
    page_size: usize,
    line_offset: &mut usize,
    item_heights: &[usize],
    viewport_height: usize,
) {
    if comment_heights_ready(len, item_heights, viewport_height) {
        page_down_with_heights(state, item_heights, viewport_height);
        ensure_comment_line_offset(state, line_offset, item_heights, viewport_height);
    } else {
        page_down(state, len, page_size);
    }
}

fn page_up_comment_list(
    state: &mut ListState,
    len: usize,
    page_size: usize,
    line_offset: &mut usize,
    item_heights: &[usize],
    viewport_height: usize,
) {
    if comment_heights_ready(len, item_heights, viewport_height) {
        page_up_with_heights(state, item_heights, viewport_height);
        ensure_comment_line_offset(state, line_offset, item_heights, viewport_height);
    } else {
        page_up(state, page_size);
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
    client.cleanup_disk_cache_background(Duration::from_secs(60 * 60 * 24));

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
