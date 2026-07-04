use super::{
    App, AppEvent, InFlightPrefetch, LoadTarget, StoriesLoadMode, View, IDLE_PREFETCH_DELAY,
    MAX_COMMENT_PREFETCH_IN_FLIGHT, PREFETCH_LOOKAHEAD,
};
use crate::api::types::{CommentNode, Story};
use crate::logging;
use std::collections::HashMap;

struct PrefetchCandidate {
    story: Story,
    priority: u32,
}

impl App {
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
        let story_ids = self.story_ids.clone();
        let page_size = self.cli.page_size;
        let feed = self.current_feed;
        self.spawn_load_detached(
            LoadTarget::Stories,
            generation,
            async move {
                client
                    .fetch_more_stories(feed, &story_ids, loaded, page_size)
                    .await
            },
            move |stories| AppEvent::StoriesLoaded {
                generation,
                mode: StoriesLoadMode::Append,
                story_ids: None,
                stories,
            },
        );
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

        let top_ids: Vec<u64> = candidates
            .iter()
            .take(MAX_COMMENT_PREFETCH_IN_FLIGHT)
            .map(|c| c.story.id)
            .collect();

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

        let selected = self.story_list_state.selected().unwrap_or(0);
        let max_cached_distance = self
            .prefetched_comments_cache
            .max_cached_distance_when_full(&self.stories, selected);

        for candidate in candidates {
            if self.comment_prefetch_in_flight.len() >= MAX_COMMENT_PREFETCH_IN_FLIGHT {
                break;
            }
            if self
                .comment_prefetch_in_flight
                .contains_key(&candidate.story.id)
            {
                continue;
            }
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

    pub fn is_comment_prefetching_for_story(&self, story_id: u64) -> bool {
        self.comment_prefetch_in_flight.contains_key(&story_id)
    }

    pub fn has_comment_prefetch_in_flight(&self) -> bool {
        !self.comment_prefetch_in_flight.is_empty()
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

        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.priority));
        candidates
    }

    fn can_prefetch_story(&self, story: &Story) -> bool {
        if story.kids.is_empty() && story.comment_count == 0 {
            return false;
        }
        if self.prefetched_comments_cache.contains(story.id) {
            return false;
        }
        true
    }

    fn start_comment_prefetch(&mut self, story: Story) {
        let generation = self.comments_prefetch_generation.advance();

        let story_id = story.id;
        let client = self.client.clone();
        let handle = self.spawn_fetch(
            async move { client.fetch_comment_roots(&story).await },
            move |comments| AppEvent::CommentsPrefetched {
                generation,
                story_id,
                comments,
            },
            move |message| AppEvent::PrefetchError {
                generation,
                story_id,
                message,
            },
        );

        self.comment_prefetch_in_flight
            .insert(story_id, InFlightPrefetch { generation, handle });
    }
}

pub(crate) struct PrefetchCache {
    entries: HashMap<u64, Vec<CommentNode>>,
    order: Vec<u64>,
    capacity: usize,
}

impl PrefetchCache {
    pub(crate) fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "prefetch cache capacity must be > 0");
        Self {
            entries: HashMap::new(),
            order: Vec::new(),
            capacity,
        }
    }

    pub(crate) fn contains(&self, story_id: u64) -> bool {
        self.entries.contains_key(&story_id)
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }

    pub(crate) fn remove(&mut self, story_id: u64) -> Option<Vec<CommentNode>> {
        self.order.retain(|id| *id != story_id);
        self.entries.remove(&story_id)
    }

    pub(crate) fn max_cached_distance_when_full(
        &self,
        stories: &[Story],
        selected: usize,
    ) -> Option<usize> {
        if self.entries.len() < self.capacity {
            return None;
        }
        self.order
            .iter()
            .filter_map(|id| {
                stories
                    .iter()
                    .position(|story| story.id == *id)
                    .map(|pos| pos.abs_diff(selected))
            })
            .max()
    }

    pub(crate) fn insert(
        &mut self,
        story_id: u64,
        comments: Vec<CommentNode>,
        stories: &[Story],
        selected: usize,
    ) {
        while self.entries.len() >= self.capacity && !self.entries.contains_key(&story_id) {
            let Some(evict_id) = self.furthest_cached_id(stories, selected) else {
                break;
            };
            self.entries.remove(&evict_id);
            self.order.retain(|id| *id != evict_id);
        }

        self.order.retain(|id| *id != story_id);
        self.order.push(story_id);
        self.entries.insert(story_id, comments);
    }

    fn furthest_cached_id(&self, stories: &[Story], selected: usize) -> Option<u64> {
        self.order.iter().copied().max_by_key(|id| {
            stories
                .iter()
                .position(|story| story.id == *id)
                .map(|pos| pos.abs_diff(selected))
                .unwrap_or(usize::MAX)
        })
    }
}

pub(crate) fn prefetch_priority(story: &Story, distance: usize, half_viewport: usize) -> u32 {
    if distance == 0 {
        return u32::MAX;
    }

    let proximity = if distance <= half_viewport {
        (half_viewport - distance) as f64 / half_viewport as f64
    } else {
        0.0
    };

    let heat = ((story.score.max(1) as f64).ln() + (story.comment_count.max(1) as f64).ln()) / 2.0;

    (proximity * 1000.0 + heat * 10.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn story(id: u64, score: i64, comments: i64) -> Story {
        Story {
            id,
            title: format!("story {id}"),
            url: None,
            score,
            by: "alice".to_string(),
            time: 1,
            comment_count: comments,
            kids: vec![id + 100],
        }
    }

    fn stories(ids: &[u64]) -> Vec<Story> {
        ids.iter().map(|id| story(*id, 10, 10)).collect()
    }

    #[test]
    fn priority_prefers_focused_then_near_hot_stories() {
        let cold = story(1, 1, 1);
        let hot = story(2, 10_000, 500);

        assert_eq!(prefetch_priority(&cold, 0, 5), u32::MAX);
        assert!(prefetch_priority(&hot, 1, 5) > prefetch_priority(&cold, 1, 5));
        assert!(prefetch_priority(&cold, 1, 5) > prefetch_priority(&cold, 5, 5));
    }

    #[test]
    fn insert_evicts_story_furthest_from_selection() {
        let stories = stories(&[1, 2, 3]);
        let mut cache = PrefetchCache::new(2);

        cache.insert(1, Vec::new(), &stories, 0);
        cache.insert(3, Vec::new(), &stories, 0);
        cache.insert(2, Vec::new(), &stories, 0);

        assert!(cache.contains(1));
        assert!(cache.contains(2));
        assert!(!cache.contains(3));
    }

    #[test]
    fn full_cache_reports_furthest_cached_distance() {
        let stories = stories(&[1, 2, 3, 4]);
        let mut cache = PrefetchCache::new(2);

        cache.insert(1, Vec::new(), &stories, 2);
        cache.insert(4, Vec::new(), &stories, 2);

        assert_eq!(cache.max_cached_distance_when_full(&stories, 2), Some(2));
    }

    #[test]
    fn remove_clears_entry_and_order() {
        let stories = stories(&[1, 2, 3]);
        let mut cache = PrefetchCache::new(2);

        cache.insert(1, Vec::new(), &stories, 0);
        assert!(cache.remove(1).is_some());
        cache.insert(2, Vec::new(), &stories, 0);
        cache.insert(3, Vec::new(), &stories, 0);

        assert_eq!(cache.entries.len(), 2);
        assert!(!cache.contains(1));
    }
}
