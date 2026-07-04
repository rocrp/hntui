use super::{App, AppEvent, SettingsPopup};
use crate::api::FeedKind;
use crate::plugin::config::{PluginConfig, SummarizeConfig};
use std::time::Instant;

impl App {
    pub(super) fn handle_feed_filter_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;

        let Some(popup) = self.feed_filter_popup.as_mut() else {
            return;
        };

        match key.code {
            KeyCode::Esc => {
                self.feed_filter_popup = None;
            }
            KeyCode::Enter => {
                let selected_feed = FeedKind::ALL[popup.feed_cursor];
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
            KeyCode::Char('j') | KeyCode::Down => {
                if popup.feed_cursor + 1 < FeedKind::ALL.len() {
                    popup.feed_cursor += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                popup.feed_cursor = popup.feed_cursor.saturating_sub(1);
            }
            _ => {}
        }
    }

    pub(super) fn handle_settings_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};

        let Some(popup) = self.settings_popup.as_mut() else {
            return;
        };

        if popup.editing {
            let mods = key.modifiers;
            let ctrl = mods.contains(KeyModifiers::CONTROL);
            let alt = mods.contains(KeyModifiers::ALT);

            match key.code {
                KeyCode::Enter => {
                    popup.confirm_edit();
                    self.save_settings();
                }
                KeyCode::Esc => popup.cancel_edit(),
                KeyCode::Left if alt => {
                    popup.edit_cursor = popup.prev_word_boundary();
                }
                KeyCode::Left => {
                    popup.edit_cursor = popup.edit_cursor.saturating_sub(1);
                }
                KeyCode::Right if alt => {
                    popup.edit_cursor = popup.next_word_boundary();
                }
                KeyCode::Right => {
                    let len = popup.edit_buffer.chars().count();
                    if popup.edit_cursor < len {
                        popup.edit_cursor += 1;
                    }
                }
                KeyCode::Home => popup.edit_cursor = 0,
                KeyCode::End => popup.edit_cursor = popup.edit_buffer.chars().count(),
                KeyCode::Char('a') if ctrl => popup.edit_cursor = 0,
                KeyCode::Char('e') if ctrl => {
                    popup.edit_cursor = popup.edit_buffer.chars().count();
                }
                KeyCode::Backspace if ctrl || alt => {
                    popup.delete_word_backward();
                }
                KeyCode::Backspace => {
                    if popup.edit_cursor > 0 {
                        let byte = popup.cursor_byte_offset();
                        let prev_byte = popup.edit_buffer[..byte]
                            .char_indices()
                            .next_back()
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                        popup.edit_buffer.replace_range(prev_byte..byte, "");
                        popup.edit_cursor -= 1;
                    }
                }
                KeyCode::Delete => {
                    let len = popup.edit_buffer.chars().count();
                    if popup.edit_cursor < len {
                        let byte = popup.cursor_byte_offset();
                        let next_byte = popup.edit_buffer[byte..]
                            .char_indices()
                            .nth(1)
                            .map(|(i, _)| byte + i)
                            .unwrap_or(popup.edit_buffer.len());
                        popup.edit_buffer.replace_range(byte..next_byte, "");
                    }
                }
                KeyCode::Char('w') if ctrl => {
                    popup.delete_word_backward();
                }
                KeyCode::Char('u') if ctrl => {
                    let byte = popup.cursor_byte_offset();
                    popup.edit_buffer.replace_range(..byte, "");
                    popup.edit_cursor = 0;
                }
                KeyCode::Char('k') if ctrl => {
                    let byte = popup.cursor_byte_offset();
                    popup.edit_buffer.truncate(byte);
                }
                KeyCode::Char(c) if !ctrl && !alt => {
                    let byte = popup.cursor_byte_offset();
                    popup.edit_buffer.insert(byte, c);
                    popup.edit_cursor += 1;
                }
                _ => {}
            }
            return;
        }

        match (key.code, key.modifiers) {
            (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
                if popup.cursor + 1 < SettingsPopup::FIELD_COUNT {
                    popup.cursor += 1;
                }
            }
            (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => {
                popup.cursor = popup.cursor.saturating_sub(1);
            }
            (KeyCode::Enter, _) => popup.start_editing(),
            (KeyCode::Esc, _) | (KeyCode::Char('q'), KeyModifiers::NONE) => {
                self.save_settings();
                self.settings_popup = None;
            }
            _ => {}
        }
    }

    pub(super) fn save_settings(&mut self) {
        let Some(popup) = self.settings_popup.as_ref() else {
            return;
        };

        let model = popup.model.trim();
        if model.is_empty() {
            self.last_error = Some("settings: model must be non-empty".to_string());
            return;
        }
        let max_comments = match popup.max_comments.trim().parse::<usize>() {
            Ok(n) if n > 0 => n,
            Ok(_) => {
                self.last_error = Some("settings: max comments must be > 0".to_string());
                return;
            }
            Err(err) => {
                self.last_error = Some(format!("settings: invalid max comments: {err}"));
                return;
            }
        };
        let system_prompt = if popup.system_prompt.trim().is_empty() {
            "Summarize this Hacker News discussion concisely. \
             Highlight key arguments, disagreements, and consensus points."
                .to_string()
        } else {
            popup.system_prompt.clone()
        };

        let api_key = {
            let trimmed = popup.api_key.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        };
        let base_url = {
            let trimmed = popup.base_url.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        };

        let config = SummarizeConfig {
            model: model.to_string(),
            api_key,
            base_url,
            max_comments,
            system_prompt,
        };

        self.summarize_plugin.update_config(Some(config.clone()));

        let path = self
            .config_path
            .clone()
            .or_else(crate::plugin::config::default_config_path);
        if let Some(path) = path {
            let tx = self.tx.clone();
            let plugin_config = PluginConfig {
                summarize: Some(config),
            };
            tokio::spawn(async move {
                let event = if let Err(err) =
                    crate::plugin::config::save_plugin_config(&path, &plugin_config).await
                {
                    AppEvent::SettingsSaveError {
                        message: format!("{err:#}"),
                    }
                } else {
                    AppEvent::SettingsSaved
                };
                let _ = tx.send(event);
            });
        } else if let Some(popup) = self.settings_popup.as_mut() {
            popup.saved_at = Some(Instant::now());
        }
    }
}
