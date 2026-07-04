use super::list_nav::{move_selection_down, move_selection_up, page_down, page_up};
use super::{App, FeedFilterPopup, SettingsPopup, View};
use crate::api::FeedKind;
use crate::input::Action;
use crossterm::event::KeyEventKind;
use std::time::Instant;

impl App {
    pub fn handle_action(&mut self, action: Action) {
        if action == Action::ToggleHelp {
            self.help_visible = !self.help_visible;
            return;
        }
        if self.help_visible {
            if action == Action::BackOrQuit {
                self.help_visible = false;
            }
            return;
        }
        if self.summarize_plugin.is_overlay_visible() {
            match action {
                Action::BackOrQuit => self.summarize_plugin.dismiss(),
                Action::MoveDown => self.summarize_plugin.scroll_down(1),
                Action::MoveUp => self.summarize_plugin.scroll_up(1),
                Action::PageDown => {
                    let amount = self
                        .summarize_plugin
                        .content_height
                        .saturating_sub(2)
                        .max(1);
                    self.summarize_plugin.scroll_down(amount);
                }
                Action::PageUp => {
                    let amount = self
                        .summarize_plugin
                        .content_height
                        .saturating_sub(2)
                        .max(1);
                    self.summarize_plugin.scroll_up(amount);
                }
                Action::ToggleCollapse => {
                    self.summarize_plugin.copy_summary();
                }
                _ => {}
            }
            return;
        }

        match (self.view, action) {
            (View::Stories, Action::BackOrQuit) if self.search_active => {
                self.exit_search_mode();
            }
            (View::Stories, Action::BackOrQuit) => self.should_quit = true,
            (View::Comments, Action::BackOrQuit) => {
                self.view = View::Stories;
                self.maybe_prefetch_comments();
            }
            (View::Stories, Action::OpenFeedFilter) => {
                let cursor = FeedKind::ALL
                    .iter()
                    .position(|&f| f == self.current_feed)
                    .unwrap_or(0);
                self.feed_filter_popup = Some(FeedFilterPopup {
                    feed_cursor: cursor,
                });
            }
            (View::Stories, Action::OpenFilter) => {
                self.filter_input_active = true;
            }
            (View::Stories, Action::StartSearch) => {
                self.search_input_active = true;
                self.search_query.clear();
            }
            (View::Stories, Action::Refresh) if self.search_active => {
                self.submit_search();
            }
            (View::Stories, Action::Refresh) => self.refresh_stories(),
            (View::Comments, Action::Refresh) => self.refresh_comments(),

            (View::Stories, Action::Enter) => self.open_comments_for_selected_story(),
            (View::Stories, Action::OpenComments) => self.open_comments_for_selected_story(),
            (View::Stories, Action::Expand) => self.open_comments_for_selected_story(),
            (View::Stories, Action::OpenPrimaryBrowser) => {
                let seen_id = self.selected_story().map(|s| s.id);
                match self.open_selected_story_in_browser() {
                    Ok(outcome) => {
                        if let crate::browser::OpenOutcome::CopiedToClipboard = outcome {
                            self.copied_flash = Some(Instant::now());
                        }
                        if let Some(id) = seen_id {
                            self.mark_story_seen(id);
                        }
                    }
                    Err(err) => self.last_error = Some(format!("{err:#}")),
                }
            }
            (View::Stories, Action::OpenSecondaryBrowser) => {
                let seen_id = self.selected_story().map(|s| s.id);
                match self.open_selected_story_comments_in_browser() {
                    Ok(outcome) => {
                        if let crate::browser::OpenOutcome::CopiedToClipboard = outcome {
                            self.copied_flash = Some(Instant::now());
                        }
                        if let Some(id) = seen_id {
                            self.mark_story_seen(id);
                        }
                    }
                    Err(err) => self.last_error = Some(format!("{err:#}")),
                }
            }
            (View::Comments, Action::OpenPrimaryBrowser) => {
                match self.open_current_story_comments_in_browser() {
                    Ok(crate::browser::OpenOutcome::CopiedToClipboard) => {
                        self.copied_flash = Some(Instant::now());
                    }
                    Ok(crate::browser::OpenOutcome::Launched) => {}
                    Err(err) => self.last_error = Some(format!("{err:#}")),
                }
            }
            (View::Comments, Action::OpenSecondaryBrowser) => {
                match self.open_current_story_in_browser() {
                    Ok(crate::browser::OpenOutcome::CopiedToClipboard) => {
                        self.copied_flash = Some(Instant::now());
                    }
                    Ok(crate::browser::OpenOutcome::Launched) => {}
                    Err(err) => self.last_error = Some(format!("{err:#}")),
                }
            }

            (View::Stories, Action::MoveDown) => {
                let count = self.visible_story_count();
                move_selection_down(&mut self.story_list_state, count);
                self.ensure_selected_story_visible();
                self.maybe_prefetch_stories();
                self.maybe_prefetch_comments();
            }
            (View::Stories, Action::MoveUp) => {
                move_selection_up(&mut self.story_list_state);
                self.ensure_selected_story_visible();
                self.maybe_prefetch_comments();
            }
            (View::Stories, Action::PageDown) => {
                let count = self.visible_story_count();
                page_down(&mut self.story_list_state, count, self.story_page_size);
                self.maybe_prefetch_stories();
                self.maybe_prefetch_comments();
            }
            (View::Stories, Action::PageUp) => {
                page_up(&mut self.story_list_state, self.story_page_size);
                self.maybe_prefetch_comments();
            }
            (View::Stories, Action::GoTop) => {
                self.story_list_state.select(Some(0));
                *self.story_list_state.offset_mut() = 0;
                self.maybe_prefetch_comments();
            }
            (View::Stories, Action::GoBottom) => {
                let count = self.visible_story_count();
                if count > 0 {
                    self.story_list_state.select(Some(count - 1));
                    self.ensure_selected_story_visible();
                    self.maybe_prefetch_stories();
                    self.maybe_prefetch_comments();
                }
            }

            (View::Comments, Action::MoveDown) => {
                let comment_len = self.comment_list.len();
                move_selection_down(&mut self.comment_list_state, comment_len);
                self.ensure_selected_comment_visible();
            }
            (View::Comments, Action::MoveUp) => {
                move_selection_up(&mut self.comment_list_state);
                self.ensure_selected_comment_visible();
            }
            (View::Comments, Action::PageDown) => {
                self.page_down_selected_comment();
            }
            (View::Comments, Action::PageUp) => {
                self.page_up_selected_comment();
            }
            (View::Comments, Action::GoTop) => {
                self.comment_list_state.select(Some(0));
                self.ensure_selected_comment_visible();
            }
            (View::Comments, Action::GoBottom) => {
                if !self.comment_list.is_empty() {
                    self.comment_list_state
                        .select(Some(self.comment_list.len() - 1));
                    self.ensure_selected_comment_visible();
                }
            }
            (View::Comments, Action::Enter) => self.toggle_selected_comment_collapse(),
            (View::Comments, Action::Collapse) => self.collapse_selected_comment(),
            (View::Comments, Action::Expand) => self.expand_selected_comment(),
            (View::Comments, Action::ToggleCollapse) => self.toggle_selected_comment_collapse(),

            (View::Comments, Action::CopyComment) => {
                self.copy_selected_comment();
            }

            (View::Comments, Action::Summarize) => {
                self.start_summary_for_loaded_comments();
            }
            (View::Stories, Action::Summarize) => {
                self.summarize_selected_story();
            }

            (_, Action::OpenSettings) => {
                self.settings_popup =
                    Some(SettingsPopup::from_config(self.summarize_plugin.config()));
            }

            (_, _) => {}
        }
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};

        if key.kind == KeyEventKind::Release {
            return;
        }
        self.last_user_activity = Instant::now();

        if self.feed_filter_popup.is_some() {
            self.handle_feed_filter_key(key);
            return;
        }

        if self.settings_popup.is_some() {
            self.handle_settings_key(key);
            return;
        }

        if self.filter_input_active {
            match key.code {
                KeyCode::Enter => {
                    self.filter_input_active = false;
                }
                KeyCode::Esc => {
                    self.keyword_filter.clear();
                    self.filter_input_active = false;
                    self.recompute_visible_stories();
                }
                KeyCode::Backspace => {
                    self.keyword_filter.pop();
                    self.recompute_visible_stories();
                }
                KeyCode::Char(c)
                    if key.modifiers == KeyModifiers::NONE
                        || key.modifiers == KeyModifiers::SHIFT =>
                {
                    self.keyword_filter.push(c);
                    self.recompute_visible_stories();
                }
                _ => {}
            }
            return;
        }

        if self.search_input_active {
            match key.code {
                KeyCode::Enter => self.submit_search(),
                KeyCode::Esc => self.cancel_search(),
                KeyCode::Backspace => {
                    self.search_query.pop();
                }
                KeyCode::Char(c)
                    if key.modifiers == KeyModifiers::NONE
                        || key.modifiers == KeyModifiers::SHIFT =>
                {
                    self.search_query.push(c);
                }
                _ => {}
            }
            return;
        }

        if let Some(action) = self.input.on_key(key) {
            self.handle_action(action);
        }
    }
}
