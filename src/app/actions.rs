use super::list_nav::{move_selection_down, move_selection_up, page_down, page_up};
use super::{App, FeedFilterPopup, SettingsPopup, TaskTarget, View};
use crate::api::FeedKind;
use crate::input::{Action, HelpAction, InputLayer, SummaryAction};
use anyhow::Context;
use crossterm::event::KeyEventKind;
use std::time::Instant;

impl App {
    pub(crate) fn input_layer(&self) -> InputLayer {
        if self.help_visible {
            InputLayer::Help
        } else if self.summary_overlay.is_visible() {
            InputLayer::Summary
        } else if let Some(settings) = &self.settings_popup {
            if settings.editing {
                InputLayer::SettingsEditor
            } else {
                InputLayer::Settings
            }
        } else if self.feed_filter_popup.is_some() {
            InputLayer::FeedFilter
        } else if self.filter_input_active {
            InputLayer::FilterText
        } else if self.search_input_active {
            InputLayer::SearchText
        } else {
            InputLayer::View
        }
    }

    pub fn handle_action(&mut self, action: Action) {
        self.last_user_activity = Instant::now();
        match action {
            Action::Noop => return,
            Action::Help(HelpAction::Dismiss) => {
                self.help_visible = false;
                return;
            }
            Action::Summary(action) => {
                match action {
                    SummaryAction::Dismiss => {
                        self.tasks.cancel(TaskTarget::Summary);
                        self.summary_overlay.dismiss();
                    }
                    SummaryAction::ScrollDown(amount) => self.summary_overlay.scroll_down(amount),
                    SummaryAction::ScrollUp(amount) => self.summary_overlay.scroll_up(amount),
                    SummaryAction::PageDown => {
                        let amount = self.summary_overlay.page_scroll_amount();
                        self.summary_overlay.scroll_down(amount);
                    }
                    SummaryAction::PageUp => {
                        let amount = self.summary_overlay.page_scroll_amount();
                        self.summary_overlay.scroll_up(amount);
                    }
                    SummaryAction::GoTop => self.summary_overlay.go_top(),
                    SummaryAction::GoBottom => self.summary_overlay.go_bottom(),
                    SummaryAction::Copy => {
                        if let Err(error) = self.summary_overlay.copy_summary() {
                            self.last_error = Some(format!("clipboard: {error:#}"));
                        }
                    }
                    SummaryAction::OpenHelp => self.help_visible = true,
                }
                return;
            }
            Action::FeedFilter(action) => {
                self.handle_feed_filter_action(action);
                return;
            }
            Action::Settings(action) => {
                self.handle_settings_action(action);
                return;
            }
            Action::FilterInput(action) => {
                self.handle_filter_input_action(action);
                return;
            }
            Action::SearchInput(action) => {
                self.handle_search_input_action(action);
                return;
            }
            _ => {}
        }

        match (self.view, action) {
            (_, Action::OpenHelp) => self.help_visible = true,
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
            (view, action @ (Action::OpenPrimaryBrowser | Action::OpenSecondaryBrowser)) => {
                let story = match view {
                    View::Stories => self.selected_story().cloned(),
                    View::Comments => self.current_story.clone(),
                };
                let result = story
                    .as_ref()
                    .context(match view {
                        View::Stories => "no selected story",
                        View::Comments => "no current story",
                    })
                    .and_then(|story| {
                        let open_source = matches!(
                            (view, action),
                            (View::Stories, Action::OpenPrimaryBrowser)
                                | (View::Comments, Action::OpenSecondaryBrowser)
                        );
                        let hn_url =
                            || format!("https://news.ycombinator.com/item?id={}", story.id);
                        let url = if open_source {
                            story.url.clone().unwrap_or_else(hn_url)
                        } else {
                            hn_url()
                        };
                        crate::browser::open_url(&url)
                    });
                let opened = match result {
                    Ok(crate::browser::OpenOutcome::CopiedToClipboard) => {
                        self.copied_flash = Some(Instant::now());
                        true
                    }
                    Ok(crate::browser::OpenOutcome::Launched) => true,
                    Err(error) => {
                        self.last_error = Some(format!("{error:#}"));
                        false
                    }
                };
                if opened && view == View::Stories {
                    self.mark_story_seen(story.expect("browser story present").id);
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
            (View::Stories, Action::SelectStory(index)) => {
                assert!(
                    index < self.visible_story_count(),
                    "story selection out of range: {index}"
                );
                self.story_list_state.select(Some(index));
                self.ensure_selected_story_visible();
                self.maybe_prefetch_comments();
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
            (View::Comments, Action::SelectComment(index)) => {
                assert!(
                    index < self.comment_list.len(),
                    "comment selection out of range: {index}"
                );
                self.comment_list_state.select(Some(index));
                self.ensure_selected_comment_visible();
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
                self.settings_popup = Some(SettingsPopup::from_config(&self.config));
            }

            (_, _) => {}
        }
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        let layer = self.input_layer();
        let action = self.input.on_key(layer, key);
        self.handle_action(action);
    }
}
