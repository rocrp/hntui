use super::{App, AppEvent, LoadTarget, StoriesLoadMode};
use crate::api::{FeedKind, Story};
use crate::logging;

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
        self.prefetch_in_flight = false;
        self.story_list_state.select(Some(0));
        *self.story_list_state.offset_mut() = 0;
        self.recompute_visible_stories();
    }

    pub(super) fn save_story_list_state_background(&self) {
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
        tokio::spawn(async move {
            if let Err(err) = store
                .save_story_list_state(story_ids, stories, feed, seen_story_ids)
                .await
            {
                logging::log_error(format!("failed to save story list state: {err:#}"));
            }
        });
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
        let generation = self.stories_generation.advance();

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
        let count = self.cli.count;
        let feed = self.current_feed;
        self.spawn_load_detached(
            LoadTarget::Stories,
            generation,
            async move { client.fetch_initial_stories(feed, count).await },
            move |(story_ids, stories)| AppEvent::StoriesLoaded {
                generation,
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
