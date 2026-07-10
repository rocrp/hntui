use super::App;
use ratatui::layout::Rect;
use ratatui::widgets::ListState;
use std::cmp;

impl App {
    pub(super) fn ensure_selected_story_visible(&mut self) {
        let count = self.visible_story_count();
        ensure_visible(&mut self.story_list_state, count, self.story_page_size);
    }

    pub(super) fn ensure_selected_comment_visible(&mut self) {
        if let Some(selected) = self.comment_list_state.selected() {
            self.comment_layout.ensure_visible(selected);
        }
    }

    pub(super) fn page_down_selected_comment(&mut self) {
        if self.comment_list.is_empty() {
            self.comment_list_state.select(None);
            return;
        }
        let selected = self.comment_list_state.selected().unwrap_or(0);
        self.comment_list_state
            .select(Some(self.comment_layout.page_down(selected)));
    }

    pub(super) fn page_up_selected_comment(&mut self) {
        if self.comment_list.is_empty() {
            self.comment_list_state.select(None);
            return;
        }
        let selected = self.comment_list_state.selected().unwrap_or(0);
        self.comment_list_state
            .select(Some(self.comment_layout.page_up(selected)));
    }
}

pub(crate) fn move_selection_down(state: &mut ListState, len: usize) {
    if len == 0 {
        state.select(None);
        *state.offset_mut() = 0;
        return;
    }
    let selected = state.selected().unwrap_or(0);
    let next = cmp::min(selected + 1, len - 1);
    state.select(Some(next));
}

pub(crate) fn move_selection_up(state: &mut ListState) {
    let Some(selected) = state.selected() else {
        return;
    };
    state.select(Some(selected.saturating_sub(1)));
}

pub(crate) fn page_down(state: &mut ListState, len: usize, page_size: usize) {
    if len == 0 {
        state.select(None);
        *state.offset_mut() = 0;
        return;
    }
    let selected = state.selected().unwrap_or(0);
    let page_size = cmp::max(page_size, 1);
    let next = cmp::min(selected + page_size, len - 1);
    state.select(Some(next));
    ensure_visible(state, len, page_size);
}

pub(crate) fn page_up(state: &mut ListState, page_size: usize) {
    let Some(selected) = state.selected() else {
        return;
    };
    let page_size = cmp::max(page_size, 1);
    state.select(Some(selected.saturating_sub(page_size)));
    ensure_visible(state, selected + 1, page_size);
}

pub(crate) fn ensure_visible(state: &mut ListState, len: usize, page_size: usize) {
    if len == 0 {
        *state.offset_mut() = 0;
        return;
    }
    let Some(selected) = state.selected() else {
        *state.offset_mut() = 0;
        return;
    };
    let page_size = cmp::max(page_size, 1);

    let offset = state.offset();
    if selected < offset {
        *state.offset_mut() = selected;
    } else if selected >= offset + page_size {
        *state.offset_mut() = selected.saturating_sub(page_size - 1);
    }
}

pub(crate) fn rect_contains(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selected(state: &ListState) -> Option<usize> {
        state.selected()
    }

    #[test]
    fn page_down_clamps_to_last_item_and_keeps_visible() {
        let mut state = ListState::default();
        state.select(Some(1));

        page_down(&mut state, 5, 3);

        assert_eq!(selected(&state), Some(4));
        assert_eq!(state.offset(), 2);
    }

    #[test]
    fn page_up_clamps_to_first_item() {
        let mut state = ListState::default();
        state.select(Some(4));
        *state.offset_mut() = 2;

        page_up(&mut state, 10);

        assert_eq!(selected(&state), Some(0));
        assert_eq!(state.offset(), 0);
    }
}
