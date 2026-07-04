use super::comment_tree::{
    attach_children as attach_children_in_tree,
    set_children_loading as set_children_loading_in_tree, set_collapse as set_collapse_in_tree,
};
use super::{App, AppEvent, LoadTarget, StoriesLoadMode};
use crate::logging;

impl App {
    pub fn handle_app_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::StoriesLoaded {
                generation,
                mode,
                story_ids,
                stories,
            } => {
                if !self.is_current_generation(LoadTarget::Stories, generation) {
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
                self.ensure_selected_story_visible();
                self.save_story_list_state_background();
                self.maybe_prefetch_comments();
            }
            AppEvent::CommentsLoaded {
                generation,
                story_id,
                comments,
            } => {
                if !self.is_current_generation(LoadTarget::Comments, generation) {
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
                self.maybe_start_pending_summary(story_id);
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
                    self.maybe_start_pending_summary(story_id);
                    return;
                }

                let selected = self.story_list_state.selected().unwrap_or(0);
                self.prefetched_comments_cache
                    .insert(story_id, comments, &self.stories, selected);
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
                self.ensure_selected_comment_visible();
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
                if set_children_loading_in_tree(&mut self.comment_tree, parent_id, false).is_none()
                    || set_collapse_in_tree(&mut self.comment_tree, parent_id, true).is_none()
                {
                    self.last_error = Some(format!("comment not found id={parent_id}"));
                    return;
                }
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
                if !self.is_current_generation(LoadTarget::Search, generation) {
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
            AppEvent::SettingsSaved => {
                if let Some(popup) = self.settings_popup.as_mut() {
                    popup.saved_at = Some(std::time::Instant::now());
                }
                self.last_error = None;
            }
            AppEvent::SettingsSaveError { message } => {
                logging::log_error(format!("failed to save config: {message}"));
                self.last_error = Some(format!("settings: {message}"));
            }
            AppEvent::Error {
                target,
                generation,
                message,
            } => {
                match target {
                    LoadTarget::Stories => {
                        if !self.is_current_generation(target, generation) {
                            return;
                        }
                        self.story_loading = false;
                        self.prefetch_in_flight = false;
                    }
                    LoadTarget::Comments => {
                        if !self.is_current_generation(target, generation) {
                            return;
                        }
                        self.comment_loading = false;
                        self.pending_summarize_story_id = None;
                    }
                    LoadTarget::Search => {
                        if !self.is_current_generation(target, generation) {
                            return;
                        }
                        self.story_loading = false;
                    }
                }
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
                if self.awaiting_prefetch_story_id == Some(story_id) {
                    self.awaiting_prefetch_story_id = None;
                    self.comment_loading = false;
                }
                if self.pending_summarize_story_id == Some(story_id) {
                    self.pending_summarize_story_id = None;
                }
                logging::log_error(format!("prefetch error story_id={story_id}: {message}"));
                self.last_error = Some(message);
                self.maybe_prefetch_comments();
            }
        }
    }
}
