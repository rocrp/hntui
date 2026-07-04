use crate::api::{CommentNode, FeedKind, HnClient, SearchClient, Story};
use crate::input::KeyState;
use crate::logging;
use crate::plugin::config::PluginConfig;
use crate::plugin::summarize::SummarizePlugin;
use crate::plugin::PluginEvent;
use crate::state::StateStore;
use crate::Cli;
use ratatui::layout::Rect;
use ratatui::widgets::ListState;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

mod actions;
mod browser_actions;
mod comment_tree;
mod comments;
mod events;
mod list_nav;
mod mouse;
mod prefetch;
mod run;
mod search;
mod settings_actions;
mod settings_popup;
mod stories;
mod tasks;

use self::list_nav::ensure_comment_line_offset;
use self::prefetch::PrefetchCache;
pub use self::run::run;
use self::search::SavedStories;
pub use self::settings_popup::SettingsPopup;
pub use self::tasks::{Generation, LoadTarget};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Stories,
    Comments,
}

#[derive(Debug)]
pub enum AppEvent {
    StoriesLoaded {
        generation: Generation,
        mode: StoriesLoadMode,
        story_ids: Option<Vec<u64>>,
        stories: Vec<Story>,
    },
    CommentsLoaded {
        generation: Generation,
        story_id: u64,
        comments: Vec<CommentNode>,
    },
    CommentsPrefetched {
        generation: Generation,
        story_id: u64,
        comments: Vec<CommentNode>,
    },
    CommentChildrenLoaded {
        generation: Generation,
        parent_id: u64,
        children: Vec<CommentNode>,
    },
    CommentChildrenError {
        generation: Generation,
        parent_id: u64,
        message: String,
    },
    SearchResultsLoaded {
        generation: Generation,
        stories: Vec<Story>,
    },
    PluginEvent(PluginEvent),
    SettingsSaved,
    SettingsSaveError {
        message: String,
    },
    Error {
        target: LoadTarget,
        generation: Generation,
        message: String,
    },
    PrefetchError {
        generation: Generation,
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

#[derive(Default, Clone, Copy)]
pub struct LayoutAreas {
    pub list_area: Rect,
    pub frame_area: Rect,
}

const IDLE_PREFETCH_DELAY: Duration = Duration::from_millis(500);
const MAX_COMMENT_PREFETCH_IN_FLIGHT: usize = 3;
const PREFETCH_CACHE_CAP: usize = 20;
const PREFETCH_LOOKAHEAD: usize = 5;

struct InFlightPrefetch {
    generation: Generation,
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
    pub copied_flash: Option<Instant>,
    pub layout_areas: LayoutAreas,

    client: HnClient,
    cli: Cli,
    tx: mpsc::UnboundedSender<AppEvent>,
    state_store: Option<StateStore>,

    stories_generation: Generation,
    comments_generation: Generation,
    comments_prefetch_generation: Generation,
    pub prefetch_in_flight: bool,
    pub has_more_stories: bool,
    comment_prefetch_in_flight: HashMap<u64, InFlightPrefetch>,
    prefetched_comments_cache: PrefetchCache,
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
    search_generation: Generation,
    search_client: SearchClient,
    saved_stories: Option<SavedStories>,
    pending_summarize_story_id: Option<u64>,

    input: KeyState,
    should_quit: bool,
    spinner_idx: usize,
    last_user_activity: Instant,

    pending_story_selection_id: Option<u64>,

    comment_children_generation: Generation,
    comment_children_in_flight: HashMap<u64, Generation>,

    pub seen_story_ids: HashSet<u64>,
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
        let summarize_plugin = SummarizePlugin::new(summarize_config, reqwest::Client::new());

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
            copied_flash: None,
            layout_areas: LayoutAreas::default(),

            client,
            cli,
            tx,
            state_store,

            stories_generation: Generation::default(),
            comments_generation: Generation::default(),
            comments_prefetch_generation: Generation::default(),
            prefetch_in_flight: false,
            has_more_stories: true,
            comment_prefetch_in_flight: HashMap::new(),
            prefetched_comments_cache: PrefetchCache::new(PREFETCH_CACHE_CAP),
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
            search_generation: Generation::default(),
            search_client: SearchClient::new(reqwest::Client::new()),
            saved_stories: None,
            pending_summarize_story_id: None,
            input: KeyState::default(),
            should_quit: false,
            spinner_idx: 0,
            last_user_activity: Instant::now(),

            pending_story_selection_id: None,

            comment_children_generation: Generation::default(),
            comment_children_in_flight: HashMap::new(),

            seen_story_ids: HashSet::new(),
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
        if self.last_error.is_none() {
            self.last_error = logging::last_write_error();
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

    pub fn prepare_frame(&mut self, area: Rect) {
        self.layout_areas.frame_area = area;

        if let Some(height) = crate::ui::plugin_overlay::content_height(area) {
            self.summarize_plugin.content_height = height;
        }

        match self.view {
            View::Stories => {
                let (list_area, _) = crate::ui::story_list::content_areas(area);
                self.layout_areas.list_area = list_area;
                self.story_page_size = (list_area.height as usize).max(1);
            }
            View::Comments => {
                let (list_area, _) = crate::ui::comment_view::content_areas(area);
                self.layout_areas.list_area = list_area;
                let viewport_height = (list_area.height as usize).max(1);
                self.comment_viewport_height = viewport_height;

                if self.comment_list.is_empty() {
                    self.comment_item_heights.clear();
                    self.comment_line_offset = 0;
                    self.comment_page_size = viewport_height;
                    return;
                }

                self.comment_item_heights =
                    crate::ui::comment_view::measure_item_heights(self, list_area);
                let total_lines: usize = self.comment_item_heights.iter().sum();
                let average_height = if self.comment_item_heights.is_empty() {
                    1
                } else {
                    (total_lines / self.comment_item_heights.len()).max(1)
                };
                self.comment_page_size = (viewport_height / average_height).max(1);

                if self.comment_list_state.selected().is_none() {
                    self.comment_list_state.select(Some(0));
                }
                self.ensure_comment_line_offset();
                let max_offset = total_lines.saturating_sub(viewport_height);
                self.comment_line_offset = self.comment_line_offset.min(max_offset);
            }
        }
    }
}
