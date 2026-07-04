use super::{App, AppEvent, LoadTarget};
use crate::api::Story;

pub(super) struct SavedStories {
    stories: Vec<Story>,
    story_ids: Vec<u64>,
    has_more_stories: bool,
}

impl SavedStories {
    fn capture(app: &App) -> Self {
        Self {
            stories: app.stories.clone(),
            story_ids: app.story_ids.clone(),
            has_more_stories: app.has_more_stories,
        }
    }
}

impl App {
    pub(super) fn submit_search(&mut self) {
        self.search_input_active = false;
        let query = self.search_query.trim().to_string();
        if query.is_empty() {
            return;
        }

        if !self.search_active {
            self.saved_stories = Some(SavedStories::capture(self));
        }

        self.search_active = true;
        self.story_loading = true;
        self.last_error = None;
        let generation = self.search_generation.advance();

        let client = self.search_client.clone();
        self.spawn_load_detached(
            LoadTarget::Search,
            generation,
            async move { client.search_stories(&query, 0).await },
            move |(stories, _has_more)| AppEvent::SearchResultsLoaded {
                generation,
                stories,
            },
        );
    }

    pub(super) fn cancel_search(&mut self) {
        self.search_input_active = false;
        self.search_query.clear();
    }

    pub(super) fn exit_search_mode(&mut self) {
        self.search_active = false;
        self.search_input_active = false;
        self.search_query.clear();

        if let Some(saved) = self.saved_stories.take() {
            self.stories = saved.stories;
            self.story_ids = saved.story_ids;
            self.has_more_stories = saved.has_more_stories;
            self.story_list_state.select(Some(0));
            *self.story_list_state.offset_mut() = 0;
        }
    }
}
