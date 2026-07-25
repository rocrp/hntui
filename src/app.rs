use crate::api::{CommentNode, FeedKind, Sources, Story, StoryThread};
use crate::article::{Article, ArticleFetcher};
use crate::config::Config;
use crate::input::KeyState;
use crate::logging;
use crate::state::StateStore;
use crate::summarizer::{Summarizer, SummaryEvent};
use crate::ui::summary_overlay::{SummaryOverlay, SummaryState};
use crate::Cli;
use ratatui::layout::Rect;
use ratatui::widgets::ListState;
use std::collections::HashSet;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

mod actions;
#[cfg(test)]
mod article_tests;
mod articles;
mod comment_tree;
mod comments;
mod events;
#[cfg(test)]
mod help_tests;
mod list_nav;
mod mouse;
mod prefetch;
mod run;
mod search;
mod settings_actions;
mod settings_popup;
mod stories;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

use self::articles::ArticleStore;
use self::prefetch::PrefetchCache;
pub use self::run::run;
use self::search::SavedStories;
pub use self::settings_popup::SettingsPopup;
use crate::tasks::TaskLifecycle;
pub(crate) use crate::tasks::{TaskId, TaskTarget};
use crate::ui::article_overlay::ArticleOverlay;
use crate::ui::comment_layout::CommentLayout;
use crate::ui::help::{HelpFocus, HelpOverlay};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Stories,
    Comments,
}

#[derive(Debug)]
pub enum AppEvent {
    StoriesLoaded {
        task: TaskId,
        mode: StoriesLoadMode,
        story_ids: Option<Vec<u64>>,
        stories: Vec<Story>,
    },
    CommentsLoaded {
        task: TaskId,
        kind: CommentLoadKind,
        thread: StoryThread,
    },
    CommentChildrenLoaded {
        task: TaskId,
        children: Vec<CommentNode>,
    },
    SearchResultsLoaded {
        task: TaskId,
        stories: Vec<Story>,
    },
    ArticleLoaded {
        task: TaskId,
        story_id: u64,
        article: Article,
    },
    Summary {
        task: TaskId,
        event: SummaryEvent,
    },
    SettingsSaved {
        task: TaskId,
        config: Config,
    },
    TaskCompleted {
        task: TaskId,
    },
    TaskFailed {
        task: TaskId,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoriesLoadMode {
    Replace,
    Append,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentLoadKind {
    Foreground,
    Prefetch,
}

/// A summarize waiting on its inputs. The two legs settle independently — the
/// article fetch and the comment load run in parallel — and the LLM call goes
/// out once neither is outstanding.
#[derive(Debug)]
pub(crate) struct PendingSummary {
    story_id: u64,
    comments_ready: bool,
    article: ArticleLeg,
}

#[derive(Debug)]
pub(crate) enum ArticleLeg {
    Pending,
    /// Settled. `None` means there was nothing to include — the toggle is off,
    /// or the story has no link and no body. Not a failure.
    Ready(Option<String>),
    /// Settled badly: summarize anyway, but say so.
    Failed(String),
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
const ARTICLE_CACHE_CAP: usize = 20;

pub struct App {
    pub view: View,
    pub help_overlay: HelpOverlay,
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
    pub comment_layout: CommentLayout,

    pub last_error: Option<String>,
    pub copied_flash: Option<Instant>,
    pub layout_areas: LayoutAreas,

    sources: Sources,
    cli: Cli,
    tasks: TaskLifecycle<AppEvent>,
    state_store: Option<StateStore>,

    pub has_more_stories: bool,
    prefetched_comments_cache: PrefetchCache,
    article_fetcher: ArticleFetcher,
    articles: ArticleStore,
    summarizer: Summarizer,
    pub summary_overlay: SummaryOverlay,
    pub article_overlay: ArticleOverlay,

    pub current_feed: FeedKind,
    pub feed_filter_popup: Option<FeedFilterPopup>,
    pub settings_popup: Option<SettingsPopup>,
    config: Config,
    pub keyword_filter: String,
    pub visible_story_indices: Vec<usize>,
    pub filter_input_active: bool,

    pub search_input_active: bool,
    pub search_query: String,
    pub search_active: bool,
    saved_stories: Option<SavedStories>,
    pending_summary: Option<PendingSummary>,

    input: KeyState,
    should_quit: bool,
    spinner_idx: usize,
    last_user_activity: Instant,

    pending_story_selection_id: Option<u64>,

    pub seen_story_ids: HashSet<u64>,
}

impl App {
    pub fn new(
        cli: Cli,
        sources: Sources,
        tx: mpsc::UnboundedSender<AppEvent>,
        state_store: Option<StateStore>,
        config: Config,
        summarizer: Summarizer,
        article_fetcher: ArticleFetcher,
    ) -> Self {
        let mut story_list_state = ListState::default();
        story_list_state.select(Some(0));

        let mut comment_list_state = ListState::default();
        comment_list_state.select(Some(0));

        Self {
            view: View::Stories,
            help_overlay: HelpOverlay::default(),
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
            comment_layout: CommentLayout::default(),

            last_error: None,
            copied_flash: None,
            layout_areas: LayoutAreas::default(),

            sources,
            cli,
            tasks: TaskLifecycle::new(
                tx,
                |task, message| AppEvent::TaskFailed { task, message },
                |task| AppEvent::TaskCompleted { task },
            ),
            state_store,

            has_more_stories: true,
            prefetched_comments_cache: PrefetchCache::new(PREFETCH_CACHE_CAP),
            article_fetcher,
            articles: ArticleStore::new(ARTICLE_CACHE_CAP),
            summarizer,
            summary_overlay: SummaryOverlay::default(),
            article_overlay: ArticleOverlay::default(),
            current_feed: FeedKind::default(),
            feed_filter_popup: None,
            settings_popup: None,
            config,
            keyword_filter: String::new(),
            visible_story_indices: vec![],
            filter_input_active: false,

            search_input_active: false,
            search_query: String::new(),
            search_active: false,
            saved_stories: None,
            pending_summary: None,
            input: KeyState::default(),
            should_quit: false,
            spinner_idx: 0,
            last_user_activity: Instant::now(),

            pending_story_selection_id: None,

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
            || self.comment_loading
            || self.tasks.is_running(TaskTarget::Stories)
            || self.tasks.is_running(TaskTarget::Search)
            || self.tasks.count_where(|target| {
                matches!(
                    target,
                    TaskTarget::CommentRoots(_)
                        | TaskTarget::CommentChildren(_)
                        | TaskTarget::Article(_)
                        | TaskTarget::Summary
                        | TaskTarget::SettingsSave
                )
            }) > 0
            || matches!(
                self.summary_overlay.state(),
                SummaryState::Loading | SummaryState::Streaming
            )
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// Which help section to highlight — the overlay in front of the view, if
    /// any. Overlays never stack, so at most one of these is visible.
    pub(crate) fn help_focus(&self) -> HelpFocus {
        if self.summary_overlay.is_visible() {
            HelpFocus::Summary
        } else if self.article_overlay.is_visible() {
            HelpFocus::Article
        } else {
            HelpFocus::View
        }
    }

    pub fn prepare_frame(&mut self, area: Rect) {
        self.layout_areas.frame_area = area;

        if let Some(viewport) = crate::ui::summary_overlay::content_area(area) {
            self.summary_overlay
                .set_viewport(viewport.width, viewport.height);
        }
        if let Some(viewport) = crate::ui::article_overlay::content_area(area) {
            self.article_overlay
                .set_viewport(viewport.width, viewport.height);
        }
        self.help_overlay
            .set_frame(area, self.view, self.help_focus());

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

                if self.comment_list.is_empty() {
                    self.comment_layout.relayout(
                        &[],
                        list_area.width as usize,
                        viewport_height,
                        self.spinner_frame(),
                    );
                    return;
                }

                let selected = self
                    .comment_list_state
                    .selected()
                    .unwrap_or(0)
                    .min(self.comment_list.len() - 1);
                self.comment_list_state.select(Some(selected));
                let spinner = self.spinner_frame();
                self.comment_layout.relayout(
                    &self.comment_list,
                    list_area.width as usize,
                    viewport_height,
                    spinner,
                );
                self.comment_layout.ensure_visible(selected);
            }
        }
    }
}
