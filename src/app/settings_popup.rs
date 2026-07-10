use crate::config::{Config, SummarizeConfig};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsField {
    Model,
    ApiKey,
    BaseUrl,
    MaxComments,
    SystemPrompt,
}

impl SettingsField {
    pub(crate) const ALL: [Self; 5] = [
        Self::Model,
        Self::ApiKey,
        Self::BaseUrl,
        Self::MaxComments,
        Self::SystemPrompt,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Model => "Model",
            Self::ApiKey => "API Key",
            Self::BaseUrl => "Base URL",
            Self::MaxComments => "Max Comments",
            Self::SystemPrompt => "System Prompt",
        }
    }

    pub(crate) fn is_secret(self) -> bool {
        self == Self::ApiKey
    }
}

pub struct SettingsPopup {
    pub cursor: usize,
    pub editing: bool,
    pub edit_buffer: String,
    pub edit_cursor: usize,
    pub model: String,
    pub api_key: String,
    pub base_url: String,
    pub max_comments: String,
    pub system_prompt: String,
    pub api_key_status: Option<String>,
    pub dirty: bool,
    pub saved_at: Option<Instant>,
}

impl SettingsPopup {
    pub const FIELD_COUNT: usize = SettingsField::ALL.len();

    pub fn from_config(config: &Config) -> Self {
        Self::from_summarize(config.summarize(), config.effective_api_key().status())
    }

    fn from_summarize(config: Option<&SummarizeConfig>, api_key_status: Option<String>) -> Self {
        match config {
            Some(c) => Self {
                cursor: 0,
                editing: false,
                edit_buffer: String::new(),
                edit_cursor: 0,
                model: c.model.clone(),
                api_key: c.api_key.clone().unwrap_or_default(),
                base_url: c.base_url.clone().unwrap_or_default(),
                max_comments: c.max_comments.to_string(),
                system_prompt: c.system_prompt.clone(),
                api_key_status,
                dirty: false,
                saved_at: None,
            },
            None => Self {
                cursor: 0,
                editing: false,
                edit_buffer: String::new(),
                edit_cursor: 0,
                model: String::new(),
                api_key: String::new(),
                base_url: String::new(),
                max_comments: "200".to_string(),
                system_prompt: String::new(),
                api_key_status,
                dirty: false,
                saved_at: None,
            },
        }
    }

    pub(crate) fn fields() -> &'static [SettingsField; Self::FIELD_COUNT] {
        &SettingsField::ALL
    }

    pub(crate) fn selected_field(&self) -> SettingsField {
        Self::fields()[self.cursor]
    }

    pub(crate) fn field_value(&self, field: SettingsField) -> &str {
        match field {
            SettingsField::Model => &self.model,
            SettingsField::ApiKey => &self.api_key,
            SettingsField::BaseUrl => &self.base_url,
            SettingsField::MaxComments => &self.max_comments,
            SettingsField::SystemPrompt => &self.system_prompt,
        }
    }

    fn field_mut(&mut self, field: SettingsField) -> &mut String {
        match field {
            SettingsField::Model => &mut self.model,
            SettingsField::ApiKey => &mut self.api_key,
            SettingsField::BaseUrl => &mut self.base_url,
            SettingsField::MaxComments => &mut self.max_comments,
            SettingsField::SystemPrompt => &mut self.system_prompt,
        }
    }

    pub fn start_editing(&mut self) {
        self.editing = true;
        self.edit_buffer = self.field_value(self.selected_field()).to_string();
        self.edit_cursor = self.edit_buffer.chars().count();
    }

    pub fn confirm_edit(&mut self) {
        let val = self.edit_buffer.clone();
        let field = self.selected_field();
        if self.field_value(field) != val {
            *self.field_mut(field) = val;
            self.dirty = true;
            self.saved_at = None;
        }
        self.editing = false;
        self.edit_buffer.clear();
        self.edit_cursor = 0;
    }

    pub(crate) fn mark_saved(&mut self) {
        self.dirty = false;
        self.saved_at = Some(Instant::now());
    }

    pub fn cancel_edit(&mut self) {
        self.editing = false;
        self.edit_buffer.clear();
        self.edit_cursor = 0;
    }

    pub(crate) fn cursor_byte_offset(&self) -> usize {
        self.edit_buffer
            .char_indices()
            .nth(self.edit_cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.edit_buffer.len())
    }

    pub(crate) fn prev_word_boundary(&self) -> usize {
        if self.edit_cursor == 0 {
            return 0;
        }
        let chars: Vec<char> = self.edit_buffer.chars().collect();
        let mut i = self.edit_cursor;
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
        i
    }

    pub(crate) fn next_word_boundary(&self) -> usize {
        let len = self.edit_buffer.chars().count();
        if self.edit_cursor >= len {
            return len;
        }
        let chars: Vec<char> = self.edit_buffer.chars().collect();
        let mut i = self.edit_cursor;
        while i < len && !chars[i].is_whitespace() {
            i += 1;
        }
        while i < len && chars[i].is_whitespace() {
            i += 1;
        }
        i
    }

    pub(crate) fn delete_word_backward(&mut self) {
        let target = self.prev_word_boundary();
        if target == self.edit_cursor {
            return;
        }
        let byte_start = self
            .edit_buffer
            .char_indices()
            .nth(target)
            .map(|(i, _)| i)
            .unwrap_or(self.edit_buffer.len());
        let byte_end = self.cursor_byte_offset();
        self.edit_buffer.replace_range(byte_start..byte_end, "");
        self.edit_cursor = target;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_confirm_updates_selected_field() {
        let mut popup = SettingsPopup::from_summarize(None, None);
        popup.cursor = 0;
        popup.start_editing();
        popup.edit_buffer = "openai/gpt-4o-mini".to_string();
        popup.edit_cursor = popup.edit_buffer.chars().count();

        popup.confirm_edit();

        assert_eq!(popup.model, "openai/gpt-4o-mini");
        assert!(!popup.editing);
        assert!(popup.edit_buffer.is_empty());
        assert!(popup.dirty);
    }

    #[test]
    fn unchanged_edit_does_not_mark_dirty() {
        let mut popup = SettingsPopup::from_summarize(None, None);
        popup.model = "gemini/gemini-flash-lite-latest".to_string();
        popup.cursor = 0;
        popup.start_editing();

        popup.confirm_edit();

        assert_eq!(popup.model, "gemini/gemini-flash-lite-latest");
        assert!(!popup.dirty);
    }

    #[test]
    fn word_boundaries_handle_unicode() {
        let mut popup = SettingsPopup::from_summarize(None, None);
        popup.edit_buffer = "alpha βeta gamma".to_string();
        popup.edit_cursor = popup.edit_buffer.chars().count();

        assert_eq!(popup.prev_word_boundary(), 11);
        popup.edit_cursor = 6;
        assert_eq!(popup.next_word_boundary(), 11);
    }

    #[test]
    fn delete_word_backward_removes_previous_word_without_breaking_utf8() {
        let mut popup = SettingsPopup::from_summarize(None, None);
        popup.edit_buffer = "hello 世界".to_string();
        popup.edit_cursor = popup.edit_buffer.chars().count();

        popup.delete_word_backward();

        assert_eq!(popup.edit_buffer, "hello ");
        assert_eq!(popup.edit_cursor, 6);
    }
}
