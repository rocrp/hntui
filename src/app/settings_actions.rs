use super::{App, AppEvent, SettingsPopup};
use crate::api::FeedKind;
use crate::app::TaskTarget;
use crate::config::{default_system_prompt, ConfigEdits, SummarizeConfig};
use crate::input::{step_bounded, CursorStep, FeedFilterAction, SettingsAction, TextAction};

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

    pub(super) fn handle_settings_action(&mut self, action: SettingsAction) {
        match action {
            SettingsAction::CloseAndSave => {
                self.save_settings();
                self.settings_popup = None;
            }
            SettingsAction::MoveDown => {
                let popup = self
                    .settings_popup
                    .as_mut()
                    .expect("settings action without popup");
                step_bounded(
                    &mut popup.cursor,
                    CursorStep::Next,
                    SettingsPopup::FIELD_COUNT,
                );
            }
            SettingsAction::MoveUp => {
                let popup = self
                    .settings_popup
                    .as_mut()
                    .expect("settings action without popup");
                step_bounded(
                    &mut popup.cursor,
                    CursorStep::Previous,
                    SettingsPopup::FIELD_COUNT,
                );
            }
            SettingsAction::StartEditing => self
                .settings_popup
                .as_mut()
                .expect("settings action without popup")
                .start_editing(),
            SettingsAction::Edit(action) => self.handle_settings_text_action(action),
        }
    }

    fn handle_settings_text_action(&mut self, action: TextAction) {
        let save = {
            let popup = self
                .settings_popup
                .as_mut()
                .expect("settings text action without popup");
            assert!(popup.editing, "settings text action outside editor");
            match action {
                TextAction::Submit => {
                    popup.confirm_edit();
                    true
                }
                TextAction::Cancel => {
                    popup.cancel_edit();
                    false
                }
                TextAction::MoveWordLeft => {
                    popup.edit_cursor = popup.prev_word_boundary();
                    false
                }
                TextAction::MoveLeft => {
                    popup.edit_cursor = popup.edit_cursor.saturating_sub(1);
                    false
                }
                TextAction::MoveWordRight => {
                    popup.edit_cursor = popup.next_word_boundary();
                    false
                }
                TextAction::MoveRight => {
                    let length = popup.edit_buffer.chars().count();
                    popup.edit_cursor = (popup.edit_cursor + 1).min(length);
                    false
                }
                TextAction::MoveToStart => {
                    popup.edit_cursor = 0;
                    false
                }
                TextAction::MoveToEnd => {
                    popup.edit_cursor = popup.edit_buffer.chars().count();
                    false
                }
                TextAction::DeleteWordBackward => {
                    popup.delete_word_backward();
                    false
                }
                TextAction::DeleteBackward => {
                    if popup.edit_cursor > 0 {
                        let byte = popup.cursor_byte_offset();
                        let previous = popup.edit_buffer[..byte]
                            .char_indices()
                            .next_back()
                            .map(|(index, _)| index)
                            .unwrap_or(0);
                        popup.edit_buffer.replace_range(previous..byte, "");
                        popup.edit_cursor -= 1;
                    }
                    false
                }
                TextAction::DeleteForward => {
                    let length = popup.edit_buffer.chars().count();
                    if popup.edit_cursor < length {
                        let byte = popup.cursor_byte_offset();
                        let next = popup.edit_buffer[byte..]
                            .char_indices()
                            .nth(1)
                            .map(|(index, _)| byte + index)
                            .unwrap_or(popup.edit_buffer.len());
                        popup.edit_buffer.replace_range(byte..next, "");
                    }
                    false
                }
                TextAction::DeleteToStart => {
                    let byte = popup.cursor_byte_offset();
                    popup.edit_buffer.replace_range(..byte, "");
                    popup.edit_cursor = 0;
                    false
                }
                TextAction::DeleteToEnd => {
                    let byte = popup.cursor_byte_offset();
                    popup.edit_buffer.truncate(byte);
                    false
                }
                TextAction::Insert(character) => {
                    let byte = popup.cursor_byte_offset();
                    popup.edit_buffer.insert(byte, character);
                    popup.edit_cursor += 1;
                    false
                }
            }
        };
        if save {
            self.save_settings();
        }
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
            _ => unreachable!("unsupported filter text action: {action:?}"),
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
            _ => unreachable!("unsupported search text action: {action:?}"),
        }
    }

    pub(super) fn save_settings(&mut self) {
        let Some(popup) = self.settings_popup.as_ref() else {
            return;
        };
        if !popup.dirty {
            return;
        }

        let model = popup.model.trim();
        if model.is_empty() {
            self.last_error = Some("settings: model must be non-empty".to_string());
            return;
        }
        let max_comments = match parse_positive(&popup.max_comments) {
            Ok(number) => number,
            Err(reason) => {
                self.last_error = Some(format!("settings: max comments {reason}"));
                return;
            }
        };
        let max_article_chars = match parse_positive(&popup.max_article_chars) {
            Ok(number) => number,
            Err(reason) => {
                self.last_error = Some(format!("settings: max article chars {reason}"));
                return;
            }
        };
        let Some(include_article) = parse_bool(&popup.include_article) else {
            self.last_error = Some("settings: include article must be true or false".to_string());
            return;
        };
        let system_prompt = if popup.system_prompt.trim().is_empty() {
            default_system_prompt()
        } else {
            popup.system_prompt.clone()
        };
        let api_key = nonempty_owned(&popup.api_key);
        let base_url = nonempty_owned(&popup.base_url);
        let summarize = SummarizeConfig {
            model: model.to_string(),
            api_key,
            base_url,
            max_comments,
            include_article,
            max_article_chars,
            system_prompt,
        };

        let current = self.config.clone();
        self.tasks.spawn(
            TaskTarget::SettingsSave,
            async move { current.save(ConfigEdits { summarize }).await },
            |task, config| AppEvent::SettingsSaved { task, config },
        );
    }
}

fn nonempty_owned(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn parse_positive(value: &str) -> Result<usize, String> {
    match value.trim().parse::<usize>() {
        Ok(number) if number > 0 => Ok(number),
        Ok(_) => Err("must be > 0".to_string()),
        Err(error) => Err(format!("is invalid: {error}")),
    }
}

/// The settings popup is a text editor, so the boolean row accepts the words
/// people actually type for it.
fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_boolean_row_accepts_the_usual_spellings() {
        for yes in ["true", "TRUE", " yes ", "on", "1"] {
            assert_eq!(parse_bool(yes), Some(true), "for {yes:?}");
        }
        for no in ["false", "No", "off", "0"] {
            assert_eq!(parse_bool(no), Some(false), "for {no:?}");
        }
        for junk in ["", "maybe", "2"] {
            assert_eq!(parse_bool(junk), None, "for {junk:?}");
        }
    }

    #[test]
    fn positive_numbers_reject_zero_and_junk() {
        assert_eq!(parse_positive(" 20000 "), Ok(20_000));
        assert!(parse_positive("0").is_err());
        assert!(parse_positive("many").is_err());
    }
}
