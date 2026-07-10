use super::comment_tree::{
    attach_children as attach_children_in_tree,
    set_children_loading as set_children_loading_in_tree, set_collapse as set_collapse_in_tree,
};
use super::{App, AppEvent, CommentLoadKind, StoriesLoadMode, TaskId, TaskTarget};
use crate::logging;

impl App {
    pub fn handle_app_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::StoriesLoaded {
                task,
                mode,
                story_ids,
                stories,
            } => self.handle_stories_loaded(task, mode, story_ids, stories),
            AppEvent::CommentsLoaded {
                task,
                kind,
                comments,
            } => self.handle_comments_loaded(task, kind, comments),
            AppEvent::CommentChildrenLoaded { task, children } => {
                if !self.tasks.finish(task) {
                    return;
                }
                let TaskTarget::CommentChildren(parent_id) = task.target() else {
                    unreachable!("comment-children event has a non-children target");
                };
                if attach_children_in_tree(&mut self.comment_tree, parent_id, children).is_none() {
                    self.last_error = Some(format!("comment not found id={parent_id}"));
                    return;
                }
                self.rebuild_comment_list(Some(parent_id));
            }
            AppEvent::SearchResultsLoaded { task, stories } => {
                if !self.tasks.finish(task) {
                    return;
                }
                assert_eq!(task.target(), TaskTarget::Search);
                self.story_loading = false;
                self.last_error = None;
                self.stories = stories;
                self.story_ids = self.stories.iter().map(|story| story.id).collect();
                self.has_more_stories = false;
                self.story_list_state.select(Some(0));
                *self.story_list_state.offset_mut() = 0;
                self.recompute_visible_stories();
            }
            AppEvent::Summary { task, event } => {
                if !self.tasks.is_current(task) {
                    return;
                }
                assert_eq!(task.target(), TaskTarget::Summary);
                self.summary_overlay.handle_event(event);
            }
            AppEvent::SettingsSaved { task, config } => {
                if !self.tasks.finish(task) {
                    return;
                }
                assert_eq!(task.target(), TaskTarget::SettingsSave);
                self.summarizer
                    .update_config(config.summarize().cloned(), config.api_key_override());
                self.config = config;
                if let Some(popup) = self.settings_popup.as_mut() {
                    popup.mark_saved();
                    popup.api_key_status = self.config.effective_api_key().status();
                }
                self.last_error = None;
            }
            AppEvent::TaskCompleted { task } => {
                self.tasks.finish(task);
            }
            AppEvent::TaskFailed { task, message } => {
                self.handle_task_failure(task, message);
            }
        }
    }

    fn handle_stories_loaded(
        &mut self,
        task: TaskId,
        mode: StoriesLoadMode,
        story_ids: Option<Vec<u64>>,
        stories: Vec<crate::api::Story>,
    ) {
        if !self.tasks.finish(task) {
            return;
        }
        assert_eq!(task.target(), TaskTarget::Stories);
        self.story_loading = false;
        self.last_error = None;
        if let Some(story_ids) = story_ids {
            self.story_ids = story_ids;
        }

        match mode {
            StoriesLoadMode::Replace => {
                self.has_more_stories = true;
                self.stories = stories;
                self.prefetched_comments_cache.clear();
                let selected = self
                    .pending_story_selection_id
                    .take()
                    .and_then(|id| self.stories.iter().position(|story| story.id == id))
                    .unwrap_or(0);
                self.story_list_state.select(Some(selected));
                *self.story_list_state.offset_mut() = 0;
            }
            StoriesLoadMode::Append => {
                if stories.is_empty() {
                    self.has_more_stories = false;
                } else {
                    for story in &stories {
                        if !self.story_ids.contains(&story.id) {
                            self.story_ids.push(story.id);
                        }
                    }
                    self.stories.extend(stories);
                }
            }
        }

        self.recompute_visible_stories();
        self.ensure_selected_story_visible();
        self.save_story_list_state_background();
        self.maybe_prefetch_comments();
    }

    fn handle_comments_loaded(
        &mut self,
        task: TaskId,
        kind: CommentLoadKind,
        comments: Vec<crate::api::CommentNode>,
    ) {
        if !self.tasks.finish(task) {
            return;
        }
        let TaskTarget::CommentRoots(story_id) = task.target() else {
            unreachable!("comments event has a non-comment-roots target");
        };
        match kind {
            CommentLoadKind::Foreground => {
                if self
                    .current_story
                    .as_ref()
                    .is_some_and(|story| story.id != story_id)
                {
                    return;
                }
                let story = self
                    .current_story
                    .clone()
                    .expect("current story present for foreground comments");
                self.apply_comments_for_story(story, comments, false);
                self.maybe_start_pending_summary(story_id);
            }
            CommentLoadKind::Prefetch => {
                let selected = self.story_list_state.selected().unwrap_or(0);
                self.prefetched_comments_cache
                    .insert(story_id, comments, &self.stories, selected);
                self.maybe_prefetch_comments();
            }
        }
    }

    fn handle_task_failure(&mut self, task: TaskId, message: String) {
        if !self.tasks.finish(task) {
            return;
        }
        logging::log_error(format!("task failed target={:?}: {message}", task.target()));
        match task.target() {
            TaskTarget::Stories | TaskTarget::Search => {
                self.story_loading = false;
                self.last_error = Some(message);
            }
            TaskTarget::CommentRoots(story_id) => {
                if self
                    .current_story
                    .as_ref()
                    .is_some_and(|story| story.id == story_id)
                {
                    self.comment_loading = false;
                }
                if self.pending_summarize_story_id == Some(story_id) {
                    self.pending_summarize_story_id = None;
                }
                self.last_error = Some(message);
                self.maybe_prefetch_comments();
            }
            TaskTarget::CommentChildren(parent_id) => {
                if set_children_loading_in_tree(&mut self.comment_tree, parent_id, false).is_none()
                    || set_collapse_in_tree(&mut self.comment_tree, parent_id, true).is_none()
                {
                    self.last_error = Some(format!("comment not found id={parent_id}"));
                    return;
                }
                self.last_error = Some(message);
                self.rebuild_comment_list(Some(parent_id));
            }
            TaskTarget::Summary => self.summary_overlay.fail(message),
            TaskTarget::SettingsSave => {
                self.last_error = Some(format!("settings: {message}"));
            }
            TaskTarget::StoryStateSave => {
                logging::log_error(format!("failed to save story state: {message}"));
            }
        }
    }
}
