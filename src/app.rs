use crate::api::{CommentNode, DiskCacheConfig, FeedKind, HnClient, SearchClient, Story};
use crate::input::{Action, KeyState};
use crate::logging;
use crate::plugin::config::{PluginConfig, SummarizeConfig};
use crate::plugin::summarize::SummarizePlugin;
use crate::plugin::{PluginContext, PluginEvent};
use crate::state::StateStore;
use crate::tui::Tui;
use crate::ui;
use crate::ui::theme;
use crate::Cli;
use anyhow::{Context, Result};
use crossterm::event::{Event, EventStream, KeyEventKind};
use futures::StreamExt;
use ratatui::layout::Rect;
use ratatui::widgets::ListState;
use std::cmp;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

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
    SearchResultsLoaded {
        generation: u64,
        stories: Vec<Story>,
    },
    PluginEvent(PluginEvent),
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

#[derive(Debug, Clone)]
pub struct FeedFilterPopup {
    pub feed_cursor: usize,
}

pub struct SettingsPopup {
    pub cursor: usize,
    pub editing: bool,
    pub edit_buffer: String,
    pub api_url: String,
    pub model: String,
    pub api_key: String,
    pub max_comments: String,
    pub system_prompt: String,
    pub saved_at: Option<Instant>,
}

impl SettingsPopup {
    pub const FIELD_COUNT: usize = 5;

    pub fn from_config(config: &Option<SummarizeConfig>) -> Self {
        match config {
            Some(c) => Self {
                cursor: 0,
                editing: false,
                edit_buffer: String::new(),
                api_url: c.api_url.clone(),
                model: c.model.clone(),
                api_key: c.api_key.clone(),
                max_comments: c.max_comments.to_string(),
                system_prompt: c.system_prompt.clone(),
                saved_at: None,
            },
            None => Self {
                cursor: 0,
                editing: false,
                edit_buffer: String::new(),
                api_url: String::new(),
                model: String::new(),
                api_key: String::new(),
                max_comments: "200".to_string(),
                system_prompt: String::new(),
                saved_at: None,
            },
        }
    }

    pub fn field_labels(&self) -> [&str; Self::FIELD_COUNT] {
        ["API URL", "Model", "API Key", "Max Comments", "System Prompt"]
    }

    pub fn field_values(&self) -> [&str; Self::FIELD_COUNT] {
        [
            &self.api_url,
            &self.model,
            &self.api_key,
            &self.max_comments,
            &self.system_prompt,
        ]
    }

    fn field_mut(&mut self, idx: usize) -> &mut String {
        match idx {
            0 => &mut self.api_url,
            1 => &mut self.model,
            2 => &mut self.api_key,
            3 => &mut self.max_comments,
            4 => &mut self.system_prompt,
            _ => unreachable!(),
        }
    }

    pub fn start_editing(&mut self) {
        self.editing = true;
        self.edit_buffer = self.field_values()[self.cursor].to_string();
    }

    pub fn confirm_edit(&mut self) {
        let val = self.edit_buffer.clone();
        *self.field_mut(self.cursor) = val;
        self.editing = false;
        self.edit_buffer.clear();
    }

    pub fn cancel_edit(&mut self) {
        self.editing = false;
        self.edit_buffer.clear();
    }
}

#[derive(Default, Clone, Copy)]
pub struct LayoutAreas {
    pub list_area: Rect,
    pub frame_area: Rect,
}

const IDLE_PREFETCH_DELAY: Duration = Duration::from_millis(500);
const MAX_COMMENT_PREFETCH_IN_FLIGHT: usize = 3;
const PREFETCH_CACHE_CAP: usize = 20;
const PREFETCH_LOOKAHEAD: usize = 5;

struct PrefetchCandidate {
    story: Story,
    priority: u32,
}

struct InFlightPrefetch {
    generation: u64,
    handle: JoinHandle<()>,
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
    pub comment_item_heights: Vec<usize>,
    pub comment_viewport_height: usize,
    pub comment_line_offset: usize,

    pub last_error: Option<String>,
    pub layout_areas: LayoutAreas,

    client: HnClient,
    cli: Cli,
    tx: mpsc::UnboundedSender<AppEvent>,
    state_store: Option<StateStore>,

    stories_generation: u64,
    comments_generation: u64,
    comments_prefetch_generation: u64,
    pub prefetch_in_flight: bool,
    pub has_more_stories: bool,
    comment_prefetch_in_flight: HashMap<u64, InFlightPrefetch>,
    prefetched_comments_cache: HashMap<u64, Vec<CommentNode>>,
    prefetch_cache_order: Vec<u64>,
    awaiting_prefetch_story_id: Option<u64>,
    pub summarize_plugin: SummarizePlugin,

    pub current_feed: FeedKind,
    pub feed_filter_popup: Option<FeedFilterPopup>,
    pub settings_popup: Option<SettingsPopup>,
    pub config_path: Option<PathBuf>,
    pub keyword_filter: String,
    pub visible_story_indices: Vec<usize>,
    pub filter_input_active: bool,

    pub search_input_active: bool,
    pub search_query: String,
    pub search_active: bool,
    search_generation: u64,
    search_client: SearchClient,
    saved_stories: Option<(Vec<Story>, Vec<u64>, bool)>,

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
        plugin_config: Option<PluginConfig>,
        config_path: Option<PathBuf>,
    ) -> Self {
        let mut story_list_state = ListState::default();
        story_list_state.select(Some(0));

        let mut comment_list_state = ListState::default();
        comment_list_state.select(Some(0));

        let summarize_config = plugin_config.and_then(|c| c.summarize);
        let summarize_plugin =
            SummarizePlugin::new(summarize_config, reqwest::Client::new());

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
            layout_areas: LayoutAreas::default(),

            client,
            cli,
            tx,
            state_store,

            stories_generation: 0,
            comments_generation: 0,
            comments_prefetch_generation: 0,
            prefetch_in_flight: false,
            has_more_stories: true,
            comment_prefetch_in_flight: HashMap::new(),
            prefetched_comments_cache: HashMap::new(),
            prefetch_cache_order: Vec::new(),
            awaiting_prefetch_story_id: None,
            summarize_plugin,
            current_feed: FeedKind::default(),
            feed_filter_popup: None,
            settings_popup: None,
            config_path,
            keyword_filter: String::new(),
            visible_story_indices: vec![],
            filter_input_active: false,

            search_input_active: false,
            search_query: String::new(),
            search_active: false,
            search_generation: 0,
            search_client: SearchClient::new(reqwest::Client::new()),
            saved_stories: None,
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
            || !self.comment_prefetch_in_flight.is_empty()
            || !self.comment_children_in_flight.is_empty()
            || matches!(
                self.summarize_plugin.state(),
                crate::plugin::summarize::SummarizeState::Loading
                    | crate::plugin::summarize::SummarizeState::Streaming
            )
    }

    pub fn restore_story_list_state(
        &mut self,
        story_ids: Vec<u64>,
        stories: Vec<Story>,
        feed: Option<FeedKind>,
    ) {
        if story_ids.is_empty() || stories.is_empty() {
            self.last_error = Some("refusing to restore empty story list state".to_string());
            return;
        }

        if let Some(f) = feed {
            self.current_feed = f;
        }
        self.story_ids = story_ids;
        self.stories = stories;
        self.story_loading = false;
        self.prefetch_in_flight = false;
        self.story_list_state.select(Some(0));
        *self.story_list_state.offset_mut() = 0;
        self.recompute_visible_stories();
    }

    fn save_story_list_state_background(&self) {
        if self.search_active {
            return;
        }
        let Some(store) = self.state_store.clone() else {
            return;
        };
        if self.story_ids.is_empty() || self.stories.is_empty() {
            return;
        }

        let story_ids = self.story_ids.clone();
        let stories = self.stories.clone();
        let feed = self.current_feed.as_str().to_string();
        tokio::spawn(async move {
            if let Err(err) = store.save_story_list_state(story_ids, stories, feed).await {
                logging::log_error(format!("failed to save story list state: {err:#}"));
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
        self.has_more_stories = true;
        for (_, inflight) in self.comment_prefetch_in_flight.drain() {
            inflight.handle.abort();
        }
        if self.stories.is_empty() {
            self.story_list_state.select(Some(0));
            *self.story_list_state.offset_mut() = 0;
        }

        let client = self.client.clone();
        let tx = self.tx.clone();
        let count = self.cli.count;
        let feed = self.current_feed;
        tokio::spawn(async move {
            match client.fetch_initial_stories(feed, count).await {
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
        if self.search_active {
            return;
        }
        if self.story_loading || self.prefetch_in_flight || !self.has_more_stories {
            return;
        }
        if self.stories.is_empty() {
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

        self.prefetch_in_flight = true;
        let generation = self.stories_generation;
        let client = self.client.clone();
        let tx = self.tx.clone();
        let story_ids = self.story_ids.clone();
        let page_size = self.cli.page_size;
        let feed = self.current_feed;
        tokio::spawn(async move {
            match client
                .fetch_more_stories(feed, &story_ids, loaded, page_size)
                .await
            {
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
        if self.story_loading {
            return;
        }
        if !self.is_idle_for_prefetch() {
            return;
        }

        let candidates = self.prefetch_story_candidates();

        // Top-N candidate IDs by priority
        let top_ids: Vec<u64> = candidates
            .iter()
            .take(MAX_COMMENT_PREFETCH_IN_FLIGHT)
            .map(|c| c.story.id)
            .collect();

        // Cancel in-flight prefetches no longer in top-N
        let to_cancel: Vec<u64> = self
            .comment_prefetch_in_flight
            .keys()
            .copied()
            .filter(|id| !top_ids.contains(id))
            .filter(|id| self.awaiting_prefetch_story_id != Some(*id))
            .collect();
        for story_id in &to_cancel {
            if let Some(inflight) = self.comment_prefetch_in_flight.remove(story_id) {
                inflight.handle.abort();
                logging::log_info(format!("cancelled prefetch story_id={story_id}"));
            }
        }

        // When the cache is full, only prefetch stories closer to the cursor than the
        // furthest cached story. Otherwise we'd evict a neighbor and immediately
        // re-prefetch it, causing an endless spinner/checkmark flashing loop.
        let max_cached_distance = if self.prefetched_comments_cache.len() >= PREFETCH_CACHE_CAP {
            let selected = self.story_list_state.selected().unwrap_or(0);
            self.prefetch_cache_order
                .iter()
                .filter_map(|id| {
                    self.stories
                        .iter()
                        .position(|s| s.id == *id)
                        .map(|pos| pos.abs_diff(selected))
                })
                .max()
        } else {
            None
        };

        let selected = self.story_list_state.selected().unwrap_or(0);
        for candidate in candidates {
            if self.comment_prefetch_in_flight.len() >= MAX_COMMENT_PREFETCH_IN_FLIGHT {
                break;
            }
            if self.comment_prefetch_in_flight.contains_key(&candidate.story.id) {
                continue;
            }
            // Skip candidates that aren't closer than the furthest cached story —
            // inserting them would evict something equally close, causing churn.
            if let Some(max_dist) = max_cached_distance {
                let candidate_dist = self
                    .stories
                    .iter()
                    .position(|s| s.id == candidate.story.id)
                    .map(|pos| pos.abs_diff(selected))
                    .unwrap_or(usize::MAX);
                if candidate_dist >= max_dist {
                    continue;
                }
            }
            self.start_comment_prefetch(candidate.story);
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
            self.prefetch_cache_order.retain(|id| *id != story.id);
            self.apply_comments_for_story(story, comments, true);
            return;
        }

        if self.comment_prefetch_in_flight.contains_key(&story.id) {
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
        if self.summarize_plugin.is_overlay_visible() {
            match action {
                Action::BackOrQuit => self.summarize_plugin.dismiss(),
                Action::MoveDown => self.summarize_plugin.scroll_down(1),
                Action::MoveUp => self.summarize_plugin.scroll_up(1),
                Action::PageDown => {
                    let amount = self.summarize_plugin.content_height.saturating_sub(2).max(1);
                    self.summarize_plugin.scroll_down(amount);
                }
                Action::PageUp => {
                    let amount = self.summarize_plugin.content_height.saturating_sub(2).max(1);
                    self.summarize_plugin.scroll_up(amount);
                }
                // 'c' key -> copy summary to clipboard
                Action::ToggleCollapse => {
                    self.summarize_plugin.copy_summary();
                }
                _ => {}
            }
            return;
        }

        match (self.view, action) {
            (View::Stories, Action::BackOrQuit) if self.search_active => {
                self.exit_search_mode();
            }
            (View::Stories, Action::BackOrQuit) => self.should_quit = true,
            (View::Comments, Action::BackOrQuit) => {
                self.view = View::Stories;
                self.maybe_prefetch_comments();
            }
            (View::Stories, Action::OpenFeedFilter) => {
                let cursor = FeedKind::ALL
                    .iter()
                    .position(|&f| f == self.current_feed)
                    .unwrap_or(0);
                self.feed_filter_popup = Some(FeedFilterPopup {
                    feed_cursor: cursor,
                });
            }
            (View::Stories, Action::OpenFilter) => {
                self.filter_input_active = true;
            }
            (View::Stories, Action::StartSearch) => {
                self.search_input_active = true;
                self.search_query.clear();
            }
            (View::Stories, Action::Refresh) if self.search_active => {
                self.submit_search();
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
                let count = self.visible_story_count();
                move_selection_down(&mut self.story_list_state, count);
                ensure_visible(
                    &mut self.story_list_state,
                    count,
                    self.story_page_size,
                );
                self.maybe_prefetch_stories();
                self.maybe_prefetch_comments();
            }
            (View::Stories, Action::MoveUp) => {
                move_selection_up(&mut self.story_list_state);
                let count = self.visible_story_count();
                ensure_visible(
                    &mut self.story_list_state,
                    count,
                    self.story_page_size,
                );
                self.maybe_prefetch_comments();
            }
            (View::Stories, Action::PageDown) => {
                let count = self.visible_story_count();
                page_down(
                    &mut self.story_list_state,
                    count,
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
                let count = self.visible_story_count();
                if count > 0 {
                    self.story_list_state.select(Some(count - 1));
                    ensure_visible(
                        &mut self.story_list_state,
                        count,
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

            (View::Comments, Action::Summarize) => {
                let ctx = PluginContext {
                    current_story: self.current_story.as_ref(),
                    comment_list: &self.comment_list,
                    tx: self.tx.clone(),
                };
                self.summarize_plugin.start(&ctx);
            }

            (_, Action::OpenSettings) => {
                self.settings_popup =
                    Some(SettingsPopup::from_config(self.summarize_plugin.config()));
            }

            (_, _) => {}
        }
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};

        if key.kind == KeyEventKind::Release {
            return;
        }
        self.last_user_activity = Instant::now();

        if self.feed_filter_popup.is_some() {
            self.handle_feed_filter_key(key);
            return;
        }

        if self.settings_popup.is_some() {
            self.handle_settings_key(key);
            return;
        }

        if self.filter_input_active {
            match key.code {
                KeyCode::Enter => {
                    self.filter_input_active = false;
                }
                KeyCode::Esc => {
                    self.keyword_filter.clear();
                    self.filter_input_active = false;
                    self.recompute_visible_stories();
                }
                KeyCode::Backspace => {
                    self.keyword_filter.pop();
                    self.recompute_visible_stories();
                }
                KeyCode::Char(c) => {
                    if key.modifiers == KeyModifiers::NONE
                        || key.modifiers == KeyModifiers::SHIFT
                    {
                        self.keyword_filter.push(c);
                        self.recompute_visible_stories();
                    }
                }
                _ => {}
            }
            return;
        }

        if self.search_input_active {
            match key.code {
                KeyCode::Enter => self.submit_search(),
                KeyCode::Esc => self.cancel_search(),
                KeyCode::Backspace => {
                    self.search_query.pop();
                }
                KeyCode::Char(c) => {
                    if key.modifiers == KeyModifiers::NONE
                        || key.modifiers == KeyModifiers::SHIFT
                    {
                        self.search_query.push(c);
                    }
                }
                _ => {}
            }
            return;
        }

        if let Some(action) = self.input.on_key(key) {
            self.handle_action(action);
        }
    }

    pub fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        use crossterm::event::{MouseButton, MouseEventKind};

        self.last_user_activity = Instant::now();
        let col = mouse.column;
        let row = mouse.row;

        // 1. Help visible → any click dismisses
        if self.help_visible {
            if matches!(
                mouse.kind,
                MouseEventKind::Down(MouseButton::Left) | MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
            ) {
                self.help_visible = false;
            }
            return;
        }

        // 2. Plugin overlay (summarize) visible
        if self.summarize_plugin.is_overlay_visible() {
            let frame_area = self.layout_areas.frame_area;
            let popup_w = (frame_area.width * 4 / 5).max(30);
            let popup_h = (frame_area.height * 4 / 5).max(10);
            let popup = crate::ui::centered(frame_area, popup_w, popup_h);

            match mouse.kind {
                MouseEventKind::ScrollDown => self.summarize_plugin.scroll_down(3),
                MouseEventKind::ScrollUp => self.summarize_plugin.scroll_up(3),
                MouseEventKind::Down(MouseButton::Left) => {
                    if !rect_contains(popup, col, row) {
                        self.summarize_plugin.dismiss();
                    }
                }
                _ => {}
            }
            return;
        }

        // 3. Settings popup — click outside dismisses
        if self.settings_popup.is_some() {
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                let frame_area = self.layout_areas.frame_area;
                let desired_width = frame_area.width.min(60);
                let line_count = SettingsPopup::FIELD_COUNT + 5; // header + blank + fields + blank + hints
                let desired_height = (line_count as u16).saturating_add(2).min(frame_area.height);
                let popup_rect =
                    crate::ui::centered(frame_area, desired_width, desired_height);
                if !rect_contains(popup_rect, col, row) {
                    self.settings_popup = None;
                }
            }
            return;
        }

        // 4. Feed filter popup
        if self.feed_filter_popup.is_some() {
            let frame_area = self.layout_areas.frame_area;
            // Reconstruct popup rect (matches feed_filter::render logic)
            let line_count = FeedKind::ALL.len() + 4; // header + blank + items + blank + hint
            let desired_width = frame_area.width.min(40);
            let desired_height = (line_count as u16).saturating_add(2).min(frame_area.height);
            let popup_rect = crate::ui::centered(frame_area, desired_width, desired_height);

            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    if !rect_contains(popup_rect, col, row) {
                        self.feed_filter_popup = None;
                    } else {
                        // Inner area (inside border)
                        let inner_y = popup_rect.y + 1; // border
                        let item_start_y = inner_y + 2; // header line + blank line
                        if row >= item_start_y && row < item_start_y + FeedKind::ALL.len() as u16 {
                            let idx = (row - item_start_y) as usize;
                            // Tap = select + confirm
                            let selected_feed = FeedKind::ALL[idx];
                            let feed_changed = selected_feed != self.current_feed;
                            self.feed_filter_popup = None;
                            if feed_changed {
                                if self.search_active {
                                    self.exit_search_mode();
                                }
                                self.current_feed = selected_feed;
                                self.refresh_stories();
                                self.recompute_visible_stories();
                            }
                        }
                    }
                }
                MouseEventKind::ScrollDown => {
                    let popup = self.feed_filter_popup.as_mut().unwrap();
                    if popup.feed_cursor + 1 < FeedKind::ALL.len() {
                        popup.feed_cursor += 1;
                    }
                }
                MouseEventKind::ScrollUp => {
                    let popup = self.feed_filter_popup.as_mut().unwrap();
                    popup.feed_cursor = popup.feed_cursor.saturating_sub(1);
                }
                _ => {}
            }
            return;
        }

        // 4. Scroll events → move selection
        match mouse.kind {
            MouseEventKind::ScrollDown => {
                self.handle_action(Action::MoveDown);
                return;
            }
            MouseEventKind::ScrollUp => {
                self.handle_action(Action::MoveUp);
                return;
            }
            _ => {}
        }

        // 5. Only handle left clicks from here; ignore during text input
        let MouseEventKind::Down(MouseButton::Left) = mouse.kind else {
            return;
        };
        if self.filter_input_active || self.search_input_active {
            return;
        }

        let list_area = self.layout_areas.list_area;

        // 6. Stories view
        if self.view == View::Stories {
            if !rect_contains(list_area, col, row) {
                return;
            }
            let offset = self.story_list_state.offset();
            let click_idx = offset + (row - list_area.y) as usize;
            let count = self.visible_story_count();
            if click_idx >= count {
                return;
            }
            let current = self.story_list_state.selected().unwrap_or(0);
            if click_idx == current {
                self.open_comments_for_selected_story();
            } else {
                self.story_list_state.select(Some(click_idx));
                ensure_visible(
                    &mut self.story_list_state,
                    count,
                    self.story_page_size,
                );
                self.maybe_prefetch_comments();
            }
            return;
        }

        // 7. Comments view
        if self.view == View::Comments {
            // Tap title bar (above list_area) → go back
            if row < list_area.y {
                self.handle_action(Action::BackOrQuit);
                return;
            }
            if !rect_contains(list_area, col, row) {
                return;
            }
            // Walk cumulative heights to find which comment was clicked
            let click_line = self.comment_line_offset + (row - list_area.y) as usize;
            let mut cumulative = 0usize;
            let mut target_idx = None;
            for (idx, &h) in self.comment_item_heights.iter().enumerate() {
                let next = cumulative + h.max(1);
                if click_line < next {
                    target_idx = Some(idx);
                    break;
                }
                cumulative = next;
            }
            let Some(idx) = target_idx else {
                return;
            };
            let current = self.comment_list_state.selected().unwrap_or(0);
            if idx == current {
                self.toggle_selected_comment_collapse();
            } else {
                self.comment_list_state.select(Some(idx));
                ensure_comment_visible(
                    &mut self.comment_list_state,
                    &mut self.comment_line_offset,
                    self.comment_list.len(),
                    &self.comment_item_heights,
                    self.comment_viewport_height,
                );
            }
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
                        self.has_more_stories = true;
                        self.stories = stories;
                        self.prefetched_comments_cache.clear();
                        self.prefetch_cache_order.clear();
                        for (_, inflight) in self.comment_prefetch_in_flight.drain() {
                            inflight.handle.abort();
                        }
                        let select_idx = self
                            .pending_story_selection_id
                            .take()
                            .and_then(|id| self.stories.iter().position(|s| s.id == id))
                            .unwrap_or(0);
                        self.story_list_state.select(Some(select_idx));
                        *self.story_list_state.offset_mut() = 0;
                    }
                    StoriesLoadMode::Append => {
                        if stories.is_empty() {
                            self.has_more_stories = false;
                        } else {
                            // Update story_ids with newly discovered IDs (for HackerWeb backend
                            // where IDs aren't known upfront).
                            for s in &stories {
                                if !self.story_ids.contains(&s.id) {
                                    self.story_ids.push(s.id);
                                }
                            }
                            self.stories.extend(stories);
                        }
                    }
                }

                self.recompute_visible_stories();
                let count = self.visible_story_count();
                ensure_visible(
                    &mut self.story_list_state,
                    count,
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
                let expected = self
                    .comment_prefetch_in_flight
                    .get(&story_id)
                    .map(|f| f.generation);
                if expected != Some(generation) {
                    return;
                }

                self.comment_prefetch_in_flight.remove(&story_id);

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

                self.insert_prefetch_cache(story_id, comments);
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
                logging::log_error(format!(
                    "comment children error parent_id={parent_id}: {message}"
                ));
                self.last_error = Some(message);
                self.rebuild_comment_list(Some(parent_id));
            }
            AppEvent::SearchResultsLoaded {
                generation,
                stories,
            } => {
                if generation != self.search_generation {
                    return;
                }
                self.story_loading = false;
                self.last_error = None;
                self.stories = stories;
                self.story_ids = self.stories.iter().map(|s| s.id).collect();
                self.has_more_stories = false;
                self.story_list_state.select(Some(0));
                *self.story_list_state.offset_mut() = 0;
                self.recompute_visible_stories();
            }
            AppEvent::PluginEvent(event) => {
                self.summarize_plugin.handle_event(event);
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
                logging::log_error(format!("load error: {message}"));
                self.last_error = Some(message);
            }
            AppEvent::PrefetchError {
                generation,
                story_id,
                message,
            } => {
                let expected = self
                    .comment_prefetch_in_flight
                    .get(&story_id)
                    .map(|f| f.generation);
                if expected != Some(generation) {
                    return;
                }
                self.comment_prefetch_in_flight.remove(&story_id);
                if self.awaiting_prefetch_story_id.is_some() {
                    self.awaiting_prefetch_story_id = None;
                    self.comment_loading = false;
                }
                logging::log_error(format!("prefetch error story_id={story_id}: {message}"));
                self.last_error = Some(message);
                self.maybe_prefetch_comments();
            }
        }
    }

    pub fn selected_story(&self) -> Option<&Story> {
        let sel = self.story_list_state.selected().unwrap_or(0);
        if self.keyword_filter.is_empty() {
            self.stories.get(sel)
        } else {
            self.visible_story_indices
                .get(sel)
                .and_then(|&i| self.stories.get(i))
        }
    }

    pub fn visible_story_count(&self) -> usize {
        if self.keyword_filter.is_empty() {
            self.stories.len()
        } else {
            self.visible_story_indices.len()
        }
    }

    pub fn recompute_visible_stories(&mut self) {
        if self.keyword_filter.is_empty() {
            self.visible_story_indices.clear();
        } else {
            let re = regex::RegexBuilder::new(&self.keyword_filter)
                .case_insensitive(true)
                .build();
            self.visible_story_indices = self
                .stories
                .iter()
                .enumerate()
                .filter(|(_, s)| match &re {
                    Ok(re) => re.is_match(&s.title),
                    Err(_) => s.title.to_lowercase().contains(&self.keyword_filter.to_lowercase()),
                })
                .map(|(i, _)| i)
                .collect();
        }
        // Clamp selection
        let count = self.visible_story_count();
        if count == 0 {
            self.story_list_state.select(Some(0));
        } else {
            let sel = self.story_list_state.selected().unwrap_or(0);
            if sel >= count {
                self.story_list_state.select(Some(count - 1));
            }
        }
    }

    pub fn is_comment_prefetching_for_story(&self, story_id: u64) -> bool {
        self.comment_prefetch_in_flight.contains_key(&story_id)
    }

    pub fn has_comment_prefetch_in_flight(&self) -> bool {
        !self.comment_prefetch_in_flight.is_empty()
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

    fn prefetch_story_candidates(&self) -> Vec<PrefetchCandidate> {
        let len = self.stories.len();
        if len == 0 {
            return Vec::new();
        }

        let offset = self.story_list_state.offset().min(len);
        let page_size = self.story_page_size.max(1);
        let half_viewport = (page_size / 2).max(1);
        let selected = self.story_list_state.selected().unwrap_or(offset);

        let start = offset.saturating_sub(PREFETCH_LOOKAHEAD);
        let end = (offset + page_size + PREFETCH_LOOKAHEAD).min(len);

        let mut candidates = Vec::new();
        for idx in start..end {
            let Some(story) = self.stories.get(idx) else {
                continue;
            };
            if !self.can_prefetch_story(story) {
                continue;
            }
            let distance = idx.abs_diff(selected);
            let priority = prefetch_priority(story, distance, half_viewport);
            candidates.push(PrefetchCandidate {
                story: story.clone(),
                priority,
            });
        }

        candidates.sort_by(|a, b| b.priority.cmp(&a.priority));
        candidates
    }

    fn can_prefetch_story(&self, story: &Story) -> bool {
        if story.kids.is_empty() && story.comment_count == 0 {
            return false;
        }
        if self.prefetched_comments_cache.contains_key(&story.id) {
            return false;
        }
        // Allow in-flight stories — caller decides whether to keep or cancel them
        true
    }

    fn start_comment_prefetch(&mut self, story: Story) {
        self.comments_prefetch_generation = self.comments_prefetch_generation.wrapping_add(1);
        let generation = self.comments_prefetch_generation;

        let story_id = story.id;
        let client = self.client.clone();
        let tx = self.tx.clone();
        let handle = tokio::spawn(async move {
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

        self.comment_prefetch_in_flight
            .insert(story_id, InFlightPrefetch { generation, handle });
    }

    fn insert_prefetch_cache(&mut self, story_id: u64, comments: Vec<CommentNode>) {
        // Evict if at capacity
        while self.prefetched_comments_cache.len() >= PREFETCH_CACHE_CAP {
            let selected = self.story_list_state.selected().unwrap_or(0);
            // Find the cached story furthest from current selection (or no longer in story list)
            let evict_id = self
                .prefetch_cache_order
                .iter()
                .copied()
                .max_by_key(|id| {
                    self.stories
                        .iter()
                        .position(|s| s.id == *id)
                        .map(|pos| pos.abs_diff(selected))
                        .unwrap_or(usize::MAX) // not in list → evict first
                });
            if let Some(evict_id) = evict_id {
                self.prefetched_comments_cache.remove(&evict_id);
                self.prefetch_cache_order.retain(|id| *id != evict_id);
            } else {
                break;
            }
        }

        self.prefetch_cache_order.retain(|id| *id != story_id);
        self.prefetch_cache_order.push(story_id);
        self.prefetched_comments_cache.insert(story_id, comments);
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

    fn submit_search(&mut self) {
        self.search_input_active = false;
        let query = self.search_query.trim().to_string();
        if query.is_empty() {
            return;
        }

        if !self.search_active {
            self.saved_stories = Some((
                self.stories.clone(),
                self.story_ids.clone(),
                self.has_more_stories,
            ));
        }

        self.search_active = true;
        self.story_loading = true;
        self.last_error = None;
        self.search_generation = self.search_generation.wrapping_add(1);
        let generation = self.search_generation;

        let client = self.search_client.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            match client.search_stories(&query, 0).await {
                Ok((stories, _has_more)) => {
                    let _ = tx.send(AppEvent::SearchResultsLoaded {
                        generation,
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

    fn cancel_search(&mut self) {
        self.search_input_active = false;
        self.search_query.clear();
    }

    fn handle_feed_filter_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;

        let Some(popup) = self.feed_filter_popup.as_mut() else {
            return;
        };

        match key.code {
            KeyCode::Esc => {
                self.feed_filter_popup = None;
            }
            KeyCode::Enter => {
                let selected_feed = FeedKind::ALL[popup.feed_cursor];
                let feed_changed = selected_feed != self.current_feed;
                self.feed_filter_popup = None;

                if feed_changed {
                    if self.search_active {
                        self.exit_search_mode();
                    }
                    self.current_feed = selected_feed;
                    self.refresh_stories();
                    self.recompute_visible_stories();
                }
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if popup.feed_cursor + 1 < FeedKind::ALL.len() {
                    popup.feed_cursor += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                popup.feed_cursor = popup.feed_cursor.saturating_sub(1);
            }
            _ => {}
        }
    }

    fn handle_settings_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};

        let Some(popup) = self.settings_popup.as_mut() else {
            return;
        };

        if popup.editing {
            match key.code {
                KeyCode::Enter => popup.confirm_edit(),
                KeyCode::Esc => popup.cancel_edit(),
                KeyCode::Backspace => {
                    popup.edit_buffer.pop();
                }
                KeyCode::Char(c) => {
                    if key.modifiers == KeyModifiers::NONE
                        || key.modifiers == KeyModifiers::SHIFT
                    {
                        popup.edit_buffer.push(c);
                    }
                }
                _ => {}
            }
            return;
        }

        match (key.code, key.modifiers) {
            (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
                if popup.cursor + 1 < SettingsPopup::FIELD_COUNT {
                    popup.cursor += 1;
                }
            }
            (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => {
                popup.cursor = popup.cursor.saturating_sub(1);
            }
            (KeyCode::Enter, _) => popup.start_editing(),
            (KeyCode::Char('s'), KeyModifiers::CONTROL) => self.save_settings(),
            (KeyCode::Esc, _) | (KeyCode::Char('q'), KeyModifiers::NONE) => {
                self.settings_popup = None;
            }
            _ => {}
        }
    }

    fn save_settings(&mut self) {
        let Some(popup) = self.settings_popup.as_mut() else {
            return;
        };

        let max_comments = popup.max_comments.parse::<usize>().unwrap_or(200);
        let system_prompt = if popup.system_prompt.trim().is_empty() {
            "Summarize this Hacker News discussion concisely. \
             Highlight key arguments, disagreements, and consensus points."
                .to_string()
        } else {
            popup.system_prompt.clone()
        };

        let config = SummarizeConfig {
            api_url: popup.api_url.clone(),
            model: popup.model.clone(),
            api_key: popup.api_key.clone(),
            max_comments,
            system_prompt,
        };

        self.summarize_plugin.update_config(Some(config.clone()));
        popup.saved_at = Some(Instant::now());

        let path = self
            .config_path
            .clone()
            .or_else(crate::plugin::config::default_config_path);
        if let Some(path) = path {
            let plugin_config = PluginConfig {
                summarize: Some(config),
            };
            tokio::spawn(async move {
                if let Err(err) =
                    crate::plugin::config::save_plugin_config(&path, &plugin_config).await
                {
                    logging::log_error(format!("failed to save config: {err:#}"));
                }
            });
        }
    }

    fn exit_search_mode(&mut self) {
        self.search_active = false;
        self.search_input_active = false;
        self.search_query.clear();

        if let Some((stories, story_ids, has_more)) = self.saved_stories.take() {
            self.stories = stories;
            self.story_ids = story_ids;
            self.has_more_stories = has_more;
            self.story_list_state.select(Some(0));
            *self.story_list_state.offset_mut() = 0;
        }
    }
}

fn prefetch_priority(story: &Story, distance: usize, half_viewport: usize) -> u32 {
    if distance == 0 {
        return u32::MAX; // focused = top priority
    }

    let proximity = if distance <= half_viewport {
        (half_viewport - distance) as f64 / half_viewport as f64
    } else {
        0.0
    };

    let heat = ((story.score.max(1) as f64).ln()
        + (story.comment_count.max(1) as f64).ln())
        / 2.0;

    (proximity * 1000.0 + heat * 10.0) as u32
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

fn rect_contains(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x
        && col < rect.x + rect.width
        && row >= rect.y
        && row < rect.y + rect.height
}

pub async fn run(
    cli: Cli,
    plugin_config: Option<PluginConfig>,
    config_path: Option<PathBuf>,
) -> Result<()> {
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

    let backend = cli.resolved_backend()?;
    let base_url = cli.resolved_base_url(backend);
    let client = HnClient::new(
        base_url,
        backend,
        cli.cache_size,
        cli.concurrency,
        disk_cache,
    )?;
    client.cleanup_disk_cache_background(Duration::from_secs(60 * 60 * 24));

    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    let mut app = App::new(
        cli,
        client,
        tx.clone(),
        state_store.clone(),
        plugin_config,
        config_path,
    );

    if let Some(store) = &state_store {
        if let Some(state) = store.load_story_list_state().await? {
            let feed = state.feed.as_deref().and_then(FeedKind::from_str_opt);
            app.restore_story_list_state(state.story_ids, state.stories, feed);
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
    if let Some(store) = &state_store {
        if !app.story_ids.is_empty() && !app.stories.is_empty() {
            store
                .save_story_list_state(app.story_ids.clone(), app.stories.clone(), app.current_feed.as_str().to_string())
                .await?;
        }
    }

    Ok(())
}
