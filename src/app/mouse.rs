use super::list_nav::rect_contains;
use super::{App, View};
use crate::api::FeedKind;
use crate::input::{
    Action, FeedFilterAction, HelpAction, InputLayer, SettingsAction, SummaryAction,
};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

impl App {
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        let action = self.mouse_action(mouse);
        self.handle_action(action);
    }

    fn mouse_action(&self, mouse: MouseEvent) -> Action {
        let col = mouse.column;
        let row = mouse.row;
        match self.input_layer() {
            InputLayer::Help => match mouse.kind {
                MouseEventKind::ScrollDown => Action::Help(HelpAction::ScrollDown(3)),
                MouseEventKind::ScrollUp => Action::Help(HelpAction::ScrollUp(3)),
                MouseEventKind::Down(MouseButton::Left) => Action::Help(HelpAction::Dismiss),
                _ => Action::Noop,
            },
            InputLayer::Summary => {
                let popup = crate::ui::summary_overlay::popup_rect(self.layout_areas.frame_area);
                match mouse.kind {
                    MouseEventKind::ScrollDown => Action::Summary(SummaryAction::ScrollDown(3)),
                    MouseEventKind::ScrollUp => Action::Summary(SummaryAction::ScrollUp(3)),
                    MouseEventKind::Down(MouseButton::Left)
                        if popup.is_some_and(|popup| !rect_contains(popup, col, row)) =>
                    {
                        Action::Summary(SummaryAction::Dismiss)
                    }
                    _ => Action::Noop,
                }
            }
            InputLayer::Settings | InputLayer::SettingsEditor => {
                if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                    && crate::ui::settings::popup_rect(self.layout_areas.frame_area)
                        .is_some_and(|popup| !rect_contains(popup, col, row))
                {
                    Action::Settings(SettingsAction::CloseAndSave)
                } else {
                    Action::Noop
                }
            }
            InputLayer::FeedFilter => self.feed_filter_mouse_action(mouse),
            InputLayer::FilterText | InputLayer::SearchText => match mouse.kind {
                MouseEventKind::ScrollDown => Action::MoveDown,
                MouseEventKind::ScrollUp => Action::MoveUp,
                _ => Action::Noop,
            },
            InputLayer::View => self.view_mouse_action(mouse),
        }
    }

    fn feed_filter_mouse_action(&self, mouse: MouseEvent) -> Action {
        match mouse.kind {
            MouseEventKind::ScrollDown => Action::FeedFilter(FeedFilterAction::MoveDown),
            MouseEventKind::ScrollUp => Action::FeedFilter(FeedFilterAction::MoveUp),
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(popup) = crate::ui::feed_filter::popup_rect(self.layout_areas.frame_area)
                else {
                    return Action::Noop;
                };
                if !rect_contains(popup, mouse.column, mouse.row) {
                    return Action::FeedFilter(FeedFilterAction::Dismiss);
                }
                let item_start_y = popup.y + 3;
                if mouse.row >= item_start_y
                    && mouse.row < item_start_y + FeedKind::ALL.len() as u16
                {
                    return Action::FeedFilter(FeedFilterAction::SelectIndex(
                        (mouse.row - item_start_y) as usize,
                    ));
                }
                Action::Noop
            }
            _ => Action::Noop,
        }
    }

    fn view_mouse_action(&self, mouse: MouseEvent) -> Action {
        match mouse.kind {
            MouseEventKind::ScrollDown => return Action::MoveDown,
            MouseEventKind::ScrollUp => return Action::MoveUp,
            MouseEventKind::Down(MouseButton::Left) => {}
            _ => return Action::Noop,
        }

        let list_area = self.layout_areas.list_area;
        match self.view {
            View::Stories => {
                if !rect_contains(list_area, mouse.column, mouse.row) {
                    return Action::Noop;
                }
                let index = self.story_list_state.offset() + (mouse.row - list_area.y) as usize;
                if index >= self.visible_story_count() {
                    return Action::Noop;
                }
                if self.story_list_state.selected().unwrap_or(0) == index {
                    Action::Enter
                } else {
                    Action::SelectStory(index)
                }
            }
            View::Comments => {
                if mouse.row < list_area.y {
                    return Action::BackOrQuit;
                }
                if !rect_contains(list_area, mouse.column, mouse.row) {
                    return Action::Noop;
                }
                let Some(index) = self
                    .comment_layout
                    .hit_test((mouse.row - list_area.y) as usize)
                else {
                    return Action::Noop;
                };
                if self.comment_list_state.selected().unwrap_or(0) == index {
                    Action::ToggleCollapse
                } else {
                    Action::SelectComment(index)
                }
            }
        }
    }
}
