use super::{App, AppEvent, StoriesLoadMode, TaskTarget};
use crate::api::{FeedKind, Story};

impl App {
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
        self.story_list_state.select(Some(0));
        *self.story_list_state.offset_mut() = 0;
        self.recompute_visible_stories();
    }

    pub(super) fn save_story_list_state_background(&mut self) {
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
        let seen_story_ids: Vec<u64> = self.seen_story_ids.iter().copied().collect();
        self.tasks.spawn(
            TaskTarget::StoryStateSave,
            async move {
                store
                    .save_story_list_state(story_ids, stories, feed, seen_story_ids)
                    .await
            },
            |task, ()| AppEvent::TaskCompleted { task },
        );
    }

    pub fn is_story_seen(&self, id: u64) -> bool {
        self.seen_story_ids.contains(&id)
    }

    pub fn mark_story_seen(&mut self, id: u64) {
        if self.seen_story_ids.insert(id) {
            self.save_story_list_state_background();
        }
    }

    pub fn refresh_stories(&mut self) {
        self.pending_story_selection_id = self.selected_story().map(|s| s.id);

        self.last_error = None;
        self.story_loading = true;
        self.has_more_stories = true;
        self.tasks.cancel(TaskTarget::Search);
        self.cancel_comment_root_tasks();
        if self.stories.is_empty() {
            self.story_list_state.select(Some(0));
            *self.story_list_state.offset_mut() = 0;
        }

        let source = self.sources.stories.clone();
        let count = self.cli.count;
        let feed = self.current_feed;
        self.tasks.spawn(
            TaskTarget::Stories,
            async move { source.initial_stories(feed, count).await },
            move |task, (story_ids, stories)| AppEvent::StoriesLoaded {
                task,
                mode: StoriesLoadMode::Replace,
                story_ids: Some(story_ids),
                stories,
            },
        );
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
                    Err(_) => s
                        .title
                        .to_lowercase()
                        .contains(&self.keyword_filter.to_lowercase()),
                })
                .map(|(i, _)| i)
                .collect();
        }

        let count = self.visible_story_count();
        if count == 0 {
            self.story_list_state.select(Some(0));
        } else {
            let selected = self.story_list_state.selected().unwrap_or(0);
            if selected >= count {
                self.story_list_state.select(Some(count - 1));
            }
        }
    }
}
