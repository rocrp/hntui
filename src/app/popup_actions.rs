use super::App;
use crate::api::FeedKind;
use crate::input::{step_bounded, CursorStep, FeedFilterAction, TextAction};

impl App {
    pub(super) fn handle_feed_filter_action(&mut self, action: FeedFilterAction) {
        let popup = self
            .feed_filter_popup
            .as_mut()
            .expect("feed-filter action without popup");
        match action {
            FeedFilterAction::Dismiss => self.feed_filter_popup = None,
            FeedFilterAction::MoveDown => {
                step_bounded(
                    &mut popup.feed_cursor,
                    CursorStep::Next,
                    FeedKind::ALL.len(),
                );
            }
            FeedFilterAction::MoveUp => {
                step_bounded(
                    &mut popup.feed_cursor,
                    CursorStep::Previous,
                    FeedKind::ALL.len(),
                );
            }
            FeedFilterAction::Select => {
                let selected = FeedKind::ALL[popup.feed_cursor];
                self.select_feed(selected);
            }
            FeedFilterAction::SelectIndex(index) => {
                let selected = *FeedKind::ALL
                    .get(index)
                    .unwrap_or_else(|| panic!("feed index out of range: {index}"));
                self.select_feed(selected);
            }
        }
    }

    fn select_feed(&mut self, selected: FeedKind) {
        let changed = selected != self.current_feed;
        self.feed_filter_popup = None;
        if !changed {
            return;
        }
        if self.search_active {
            self.exit_search_mode();
        }
        self.current_feed = selected;
        self.refresh_stories();
        self.recompute_visible_stories();
    }

    pub(super) fn handle_filter_input_action(&mut self, action: TextAction) {
        assert!(self.filter_input_active, "filter action outside text input");
        match action {
            TextAction::Submit => self.filter_input_active = false,
            TextAction::Cancel => {
                self.keyword_filter.clear();
                self.filter_input_active = false;
                self.recompute_visible_stories();
            }
            TextAction::DeleteBackward => {
                self.keyword_filter.pop();
                self.recompute_visible_stories();
            }
            TextAction::Insert(character) => {
                self.keyword_filter.push(character);
                self.recompute_visible_stories();
            }
        }
    }

    pub(super) fn handle_search_input_action(&mut self, action: TextAction) {
        assert!(self.search_input_active, "search action outside text input");
        match action {
            TextAction::Submit => self.submit_search(),
            TextAction::Cancel => self.cancel_search(),
            TextAction::DeleteBackward => {
                self.search_query.pop();
            }
            TextAction::Insert(character) => self.search_query.push(character),
        }
    }
}
