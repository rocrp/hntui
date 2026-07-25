use super::articles::ArticleRequest;
use super::comment_tree::{
    apply_default_expansion, flatten_visible_comments, info_for_comment as comment_info_in_tree,
    set_children_loading as set_children_loading_in_tree, set_collapse as set_collapse_in_tree,
};
use super::{App, AppEvent, ArticleLeg, CommentLoadKind, PendingSummary, TaskTarget, View};
use crate::api::{Story, StoryThread};
use crate::article::Article;
use crate::config::default_include_article;
use crate::summarizer::SummaryInput;
use crate::ui::theme;
use anyhow::{Context, Result};
use std::time::Instant;

impl App {
    pub fn refresh_comments(&mut self) {
        let Some(story) = self.current_story.clone() else {
            self.last_error = Some("no current story".to_string());
            return;
        };
        self.load_comments_for_story(story, true);
    }

    pub fn open_comments_for_selected_story(&mut self) {
        let Some(story) = self.selected_story().cloned() else {
            return;
        };
        self.mark_story_seen(story.id);

        if self
            .current_story
            .as_ref()
            .is_some_and(|s| s.id == story.id)
            && !self.comment_tree.is_empty()
        {
            self.view = View::Comments;
            return;
        }

        if let Some(thread) = self.prefetched_comments_cache.remove(story.id) {
            self.apply_comments_for_story(story, thread, true);
            return;
        }

        self.load_comments_for_story(story, true);
    }

    pub(super) fn load_comments_for_story(&mut self, story: Story, switch_view: bool) {
        if switch_view {
            self.view = View::Comments;
        }

        self.last_error = None;
        if let Some(previous_story_id) = self
            .current_story
            .as_ref()
            .map(|current| current.id)
            .filter(|&id| id != story.id)
        {
            self.tasks
                .cancel(TaskTarget::CommentRoots(previous_story_id));
            if self
                .pending_summary
                .as_ref()
                .is_some_and(|pending| pending.story_id == previous_story_id)
            {
                self.abandon_pending_summary();
            }
        }
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
        let source = self.sources.stories.clone();
        self.tasks.spawn(
            TaskTarget::CommentRoots(story_id),
            async move { source.comment_roots(story).await },
            move |task, thread| AppEvent::CommentsLoaded {
                task,
                kind: CommentLoadKind::Foreground,
                thread,
            },
        );
    }

    pub(super) fn apply_comments_for_story(
        &mut self,
        mut story: Story,
        thread: StoryThread,
        switch_view: bool,
    ) {
        if switch_view {
            self.view = View::Comments;
        }
        self.comment_loading = false;
        self.tasks
            .cancel_where(|target| matches!(target, TaskTarget::CommentChildren(_)));
        self.last_error = None;
        story.absorb_text(thread.text);
        self.remember_story_text(&story);
        self.current_story = Some(story);
        self.comment_tree = thread.comments;
        self.apply_default_comment_expansion();
        self.rebuild_comment_list(None);
        self.comment_list_state.select(Some(0));
        self.comment_layout.invalidate();
        *self.comment_list_state.offset_mut() = 0;
    }

    /// Keep a body discovered with the discussion on the list entry too, so it
    /// survives navigation and lands in the persisted story state.
    fn remember_story_text(&mut self, story: &Story) {
        let Some(text) = story.text.clone() else {
            return;
        };
        if let Some(listed) = self
            .stories
            .iter_mut()
            .find(|listed| listed.id == story.id && listed.text.is_none())
        {
            listed.text = Some(text);
        }
    }

    pub(super) fn reset_comment_state(&mut self) {
        self.comment_tree.clear();
        self.tasks
            .cancel_where(|target| matches!(target, TaskTarget::CommentChildren(_)));
        self.comment_list.clear();
        self.comment_layout.invalidate();
        self.comment_list_state.select(Some(0));
        *self.comment_list_state.offset_mut() = 0;
    }

    pub(super) fn rebuild_comment_list(&mut self, preserve_comment_id: Option<u64>) {
        self.comment_list = flatten_visible_comments(&self.comment_tree);
        self.comment_layout.invalidate();

        let Some(id) = preserve_comment_id else {
            return;
        };
        if let Some(idx) = self.comment_list.iter().position(|c| c.id == id) {
            self.comment_list_state.select(Some(idx));
        }
    }

    fn apply_default_comment_expansion(&mut self) {
        apply_default_expansion(
            &mut self.comment_tree,
            theme::COMMENT_DEFAULT_VISIBLE_LEVELS,
        );
    }

    fn start_loading_comment_children(&mut self, parent_id: u64) {
        if self
            .tasks
            .is_running(TaskTarget::CommentChildren(parent_id))
        {
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

        if set_children_loading_in_tree(&mut self.comment_tree, parent_id, true).is_none() {
            self.last_error = Some(format!("comment not found id={parent_id}"));
            return;
        }
        if set_collapse_in_tree(&mut self.comment_tree, parent_id, false).is_none() {
            self.last_error = Some(format!("comment not found id={parent_id}"));
            return;
        }

        self.rebuild_comment_list(Some(parent_id));

        let depth = parent_depth.saturating_add(1);
        let source = self.sources.stories.clone();
        self.tasks.spawn(
            TaskTarget::CommentChildren(parent_id),
            async move { source.comment_children(kids, depth).await },
            move |task, children| AppEvent::CommentChildrenLoaded { task, children },
        );
    }

    pub(super) fn cancel_comment_root_tasks(&mut self) {
        self.tasks
            .cancel_where(|target| matches!(target, TaskTarget::CommentRoots(_)));
        self.comment_loading = false;
        self.abandon_pending_summary();
    }

    pub(super) fn copy_selected_comment(&mut self) {
        let Some(selected) = self.comment_list_state.selected() else {
            return;
        };
        let Some(comment) = self.comment_list.get(selected) else {
            return;
        };
        let plain = crate::text::hn_html_to_plain(&comment.text);
        let by = comment.by.as_deref().unwrap_or("[unknown]");
        let text = format!("{by}: {plain}");
        match copy_to_clipboard(text) {
            Ok(()) => {
                self.copied_flash = Some(Instant::now());
            }
            Err(e) => {
                self.last_error = Some(format!("clipboard: {e}"));
            }
        }
    }

    pub(super) fn collapse_selected_comment(&mut self) {
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
    }

    pub(super) fn expand_selected_comment(&mut self) {
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
    }

    pub(super) fn toggle_selected_comment_collapse(&mut self) {
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

    pub(super) fn summarize_selected_story(&mut self) {
        let Some(story) = self.selected_story().cloned() else {
            self.last_error = Some("no selected story".to_string());
            return;
        };
        self.mark_story_seen(story.id);

        let is_current = self
            .current_story
            .as_ref()
            .is_some_and(|current| current.id == story.id);
        if is_current && !self.comment_list.is_empty() {
            self.start_summary_for_loaded_comments();
            return;
        }

        if let Some(thread) = self.prefetched_comments_cache.remove(story.id) {
            self.apply_comments_for_story(story, thread, false);
            self.start_summary_for_loaded_comments();
            return;
        }

        // Comments are not in hand: the load and the article fetch run in
        // parallel, and `begin_summary` shows the overlay while they settle.
        self.begin_summary(story.clone(), false);
        self.load_comments_for_story(story, false);
    }

    pub(super) fn start_summary_for_loaded_comments(&mut self) {
        let Some(story) = self.current_story.clone() else {
            self.summary_overlay.fail("No story selected".to_string());
            return;
        };
        self.begin_summary(story, true);
    }

    fn begin_summary(&mut self, story: Story, comments_ready: bool) {
        self.summary_overlay.begin(&story, self.comment_list.len());
        let article = self.plan_article_leg(&story);
        self.pending_summary = Some(PendingSummary {
            story_id: story.id,
            comments_ready,
            article,
        });
        self.run_or_await_pending_summary();
    }

    /// Decide the article leg, kicking a fetch when one is needed. A story
    /// with nothing to fetch is settled, not failed.
    fn plan_article_leg(&mut self, story: &Story) -> ArticleLeg {
        let includes_article = self
            .config
            .summarize()
            .map_or_else(default_include_article, |summarize| {
                summarize.include_article
            });
        if !includes_article {
            return ArticleLeg::Ready(None);
        }
        match self.request_article(story) {
            ArticleRequest::Ready(article) => ArticleLeg::Ready(Some(article.content)),
            ArticleRequest::Fetching => ArticleLeg::Pending,
            ArticleRequest::Unavailable => ArticleLeg::Ready(None),
        }
    }

    /// A settled article fetch reaching a summarize that is waiting on it.
    pub(super) fn settle_pending_summary_article(
        &mut self,
        story_id: u64,
        result: Result<Article, String>,
    ) {
        let Some(pending) = self.pending_summary.as_mut() else {
            return;
        };
        if pending.story_id != story_id || !matches!(pending.article, ArticleLeg::Pending) {
            return;
        }
        pending.article = match result {
            Ok(article) => ArticleLeg::Ready(Some(article.content)),
            Err(message) => ArticleLeg::Failed(message),
        };
        self.run_or_await_pending_summary();
    }

    pub(super) fn maybe_start_pending_summary(&mut self, story_id: u64) {
        let Some(pending) = self.pending_summary.as_mut() else {
            return;
        };
        if pending.story_id != story_id {
            return;
        }
        pending.comments_ready = true;
        self.run_or_await_pending_summary();
    }

    /// Fail a pending summarize outright — its comments never arrived.
    pub(super) fn fail_pending_summary(&mut self, story_id: u64, message: &str) {
        if self
            .pending_summary
            .as_ref()
            .is_some_and(|pending| pending.story_id == story_id)
        {
            self.abandon_pending_summary();
            self.summary_overlay.fail(message.to_string());
        }
    }

    pub(super) fn abandon_pending_summary(&mut self) {
        let Some(pending) = self.pending_summary.take() else {
            return;
        };
        self.cancel_article_fetch(pending.story_id);
    }

    /// Send the summarize once both legs have settled; otherwise say what the
    /// overlay is still waiting on.
    fn run_or_await_pending_summary(&mut self) {
        let Some(pending) = self.pending_summary.as_ref() else {
            return;
        };
        let waiting = match (pending.comments_ready, &pending.article) {
            (false, ArticleLeg::Pending) => Some("fetching article and comments"),
            (false, _) => Some("loading comments"),
            (true, ArticleLeg::Pending) => Some("fetching article"),
            (true, _) => None,
        };
        if let Some(waiting) = waiting {
            self.summary_overlay
                .set_waiting_for(Some(waiting.to_string()));
            return;
        }

        let pending = self
            .pending_summary
            .take()
            .expect("pending summary present");
        let Some(story) = self
            .current_story
            .clone()
            .filter(|story| story.id == pending.story_id)
        else {
            self.summary_overlay
                .fail("story changed before the summary started".to_string());
            return;
        };

        let (article, notice) = match pending.article {
            ArticleLeg::Ready(article) => (article, None),
            ArticleLeg::Failed(reason) => (None, Some(reason)),
            ArticleLeg::Pending => unreachable!("a pending leg cannot start a summary"),
        };
        self.summary_overlay
            .set_comment_count(self.comment_list.len());
        self.summary_overlay.set_waiting_for(None);
        self.summary_overlay.set_article_notice(notice);

        let input = SummaryInput {
            story,
            comments: self.comment_list.clone(),
            article,
        };
        let summarizer = self.summarizer.clone();
        self.tasks.spawn_stream(
            TaskTarget::Summary,
            summarizer.summarize(input),
            |task, event| AppEvent::Summary { task, event },
        );
    }
}

#[cfg(not(target_os = "android"))]
fn copy_to_clipboard(text: String) -> Result<()> {
    arboard::Clipboard::new()
        .and_then(|mut cb| cb.set_text(text))
        .context("copy to clipboard")
}

#[cfg(target_os = "android")]
fn copy_to_clipboard(_text: String) -> Result<()> {
    anyhow::bail!("clipboard unavailable on Android")
}
