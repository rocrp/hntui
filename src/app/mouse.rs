use super::list_nav::rect_contains;
use super::{App, View};
use crate::api::FeedKind;
use crate::input::Action;
use std::time::Instant;

impl App {
    pub fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        use crossterm::event::{MouseButton, MouseEventKind};

        self.last_user_activity = Instant::now();
        let col = mouse.column;
        let row = mouse.row;

        // 1. Help visible -> any click dismisses
        if self.help_visible {
            if matches!(
                mouse.kind,
                MouseEventKind::Down(MouseButton::Left)
                    | MouseEventKind::ScrollDown
                    | MouseEventKind::ScrollUp
            ) {
                self.help_visible = false;
            }
            return;
        }

        // 2. Plugin overlay (summarize) visible
        if self.summarize_plugin.is_overlay_visible() {
            let frame_area = self.layout_areas.frame_area;
            let popup = crate::ui::plugin_overlay::popup_rect(frame_area);

            match mouse.kind {
                MouseEventKind::ScrollDown => self.summarize_plugin.scroll_down(3),
                MouseEventKind::ScrollUp => self.summarize_plugin.scroll_up(3),
                MouseEventKind::Down(MouseButton::Left)
                    if popup.is_some_and(|popup| !rect_contains(popup, col, row)) =>
                {
                    self.summarize_plugin.dismiss();
                }
                _ => {}
            }
            return;
        }

        // 3. Settings popup -> click outside dismisses
        if self.settings_popup.is_some() {
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                let frame_area = self.layout_areas.frame_area;
                if crate::ui::settings::popup_rect(frame_area)
                    .is_some_and(|popup_rect| !rect_contains(popup_rect, col, row))
                {
                    self.save_settings();
                    self.settings_popup = None;
                }
            }
            return;
        }

        // 4. Feed filter popup
        if self.feed_filter_popup.is_some() {
            let frame_area = self.layout_areas.frame_area;
            let popup_rect = crate::ui::feed_filter::popup_rect(frame_area);

            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    let Some(popup_rect) = popup_rect else {
                        return;
                    };
                    if !rect_contains(popup_rect, col, row) {
                        self.feed_filter_popup = None;
                    } else {
                        let inner_y = popup_rect.y + 1;
                        let item_start_y = inner_y + 2;
                        if row >= item_start_y && row < item_start_y + FeedKind::ALL.len() as u16 {
                            let idx = (row - item_start_y) as usize;
                            let selected_feed = FeedKind::ALL[idx];
                            let feed_changed = selected_feed != self.current_feed;
                            self.feed_filter_popup = None;
                            if feed_changed {
                                if self.search_active {
                                    self.exit_search_mode();
                                }
                                self.current_feed = selected_feed;
                                self.refresh_stories();
                                self.recompute_visible_stories();
                            }
                        }
                    }
                }
                MouseEventKind::ScrollDown => {
                    if let Some(popup) = self.feed_filter_popup.as_mut() {
                        if popup.feed_cursor + 1 < FeedKind::ALL.len() {
                            popup.feed_cursor += 1;
                        }
                    }
                }
                MouseEventKind::ScrollUp => {
                    if let Some(popup) = self.feed_filter_popup.as_mut() {
                        popup.feed_cursor = popup.feed_cursor.saturating_sub(1);
                    }
                }
                _ => {}
            }
            return;
        }

        match mouse.kind {
            MouseEventKind::ScrollDown => {
                self.handle_action(Action::MoveDown);
                return;
            }
            MouseEventKind::ScrollUp => {
                self.handle_action(Action::MoveUp);
                return;
            }
            _ => {}
        }

        let MouseEventKind::Down(MouseButton::Left) = mouse.kind else {
            return;
        };
        if self.filter_input_active || self.search_input_active {
            return;
        }

        let list_area = self.layout_areas.list_area;

        if self.view == View::Stories {
            if !rect_contains(list_area, col, row) {
                return;
            }
            let offset = self.story_list_state.offset();
            let click_idx = offset + (row - list_area.y) as usize;
            let count = self.visible_story_count();
            if click_idx >= count {
                return;
            }
            let current = self.story_list_state.selected().unwrap_or(0);
            if click_idx == current {
                self.open_comments_for_selected_story();
            } else {
                self.story_list_state.select(Some(click_idx));
                self.ensure_selected_story_visible();
                self.maybe_prefetch_comments();
            }
            return;
        }

        if self.view == View::Comments {
            if row < list_area.y {
                self.handle_action(Action::BackOrQuit);
                return;
            }
            if !rect_contains(list_area, col, row) {
                return;
            }

            let click_line = self.comment_line_offset + (row - list_area.y) as usize;
            let mut cumulative = 0usize;
            let mut target_idx = None;
            for (idx, &h) in self.comment_item_heights.iter().enumerate() {
                let next = cumulative + h.max(1);
                if click_line < next {
                    target_idx = Some(idx);
                    break;
                }
                cumulative = next;
            }
            let Some(idx) = target_idx else {
                return;
            };
            let current = self.comment_list_state.selected().unwrap_or(0);
            if idx == current {
                self.toggle_selected_comment_collapse();
            } else {
                self.comment_list_state.select(Some(idx));
                self.ensure_selected_comment_visible();
            }
        }
    }
}
