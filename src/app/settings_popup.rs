use crate::config::{
    default_include_article, default_max_article_chars, default_max_comments, Config,
    SummarizeConfig,
};
use std::time::{Duration, Instant};

pub(super) fn nonempty_owned(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsField {
    Model,
    ApiKey,
    BaseUrl,
    MaxComments,
    IncludeArticle,
    MaxArticleChars,
    SystemPrompt,
}

impl SettingsField {
    pub(crate) const ALL: [Self; 7] = [
        Self::Model,
        Self::ApiKey,
        Self::BaseUrl,
        Self::MaxComments,
        Self::IncludeArticle,
        Self::MaxArticleChars,
        Self::SystemPrompt,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Model => "Model",
            Self::ApiKey => "API Key",
            Self::BaseUrl => "Base URL",
            Self::MaxComments => "Max Comments",
            Self::IncludeArticle => "Include Article",
            Self::MaxArticleChars => "Max Article Chars",
            Self::SystemPrompt => "System Prompt",
        }
    }

    pub(crate) fn is_secret(self) -> bool {
        self == Self::ApiKey
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsRow {
    Field(SettingsField),
    TestConnection,
}

impl SettingsRow {
    const ALL: [Self; 8] = [
        Self::Field(SettingsField::Model),
        Self::Field(SettingsField::ApiKey),
        Self::Field(SettingsField::BaseUrl),
        Self::Field(SettingsField::MaxComments),
        Self::Field(SettingsField::IncludeArticle),
        Self::Field(SettingsField::MaxArticleChars),
        Self::Field(SettingsField::SystemPrompt),
        Self::TestConnection,
    ];
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum ConnectionTestState {
    #[default]
    Idle,
    Testing,
    Success {
        model: String,
        ttft: Duration,
    },
    Error(String),
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
    pub include_article: String,
    pub max_article_chars: String,
    pub system_prompt: String,
    pub api_key_status: Option<String>,
    pub(crate) connection_test: ConnectionTestState,
    pub dirty: bool,
    pub saved_at: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolvedEndpointPreview {
    Ready(String),
    Error(String),
}

impl ResolvedEndpointPreview {
    pub(crate) fn text(&self) -> &str {
        match self {
            Self::Ready(text) | Self::Error(text) => text,
        }
    }

    pub(crate) fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }
}

impl SettingsPopup {
    pub const FIELD_COUNT: usize = SettingsField::ALL.len();
    pub const ROW_COUNT: usize = SettingsRow::ALL.len();

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
                include_article: c.include_article.to_string(),
                max_article_chars: c.max_article_chars.to_string(),
                system_prompt: c.system_prompt.clone(),
                api_key_status,
                connection_test: ConnectionTestState::Idle,
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
                max_comments: default_max_comments().to_string(),
                include_article: default_include_article().to_string(),
                max_article_chars: default_max_article_chars().to_string(),
                system_prompt: String::new(),
                api_key_status,
                connection_test: ConnectionTestState::Idle,
                dirty: false,
                saved_at: None,
            },
        }
    }

    pub(crate) fn fields() -> &'static [SettingsField; Self::FIELD_COUNT] {
        &SettingsField::ALL
    }

    pub(crate) fn rows() -> &'static [SettingsRow; Self::ROW_COUNT] {
        &SettingsRow::ALL
    }

    pub(crate) fn selected_row(&self) -> SettingsRow {
        Self::rows()[self.cursor]
    }

    pub(crate) fn selected_field(&self) -> Option<SettingsField> {
        match self.selected_row() {
            SettingsRow::Field(field) => Some(field),
            SettingsRow::TestConnection => None,
        }
    }

    pub(crate) fn field_value(&self, field: SettingsField) -> &str {
        match field {
            SettingsField::Model => &self.model,
            SettingsField::ApiKey => &self.api_key,
            SettingsField::BaseUrl => &self.base_url,
            SettingsField::MaxComments => &self.max_comments,
            SettingsField::IncludeArticle => &self.include_article,
            SettingsField::MaxArticleChars => &self.max_article_chars,
            SettingsField::SystemPrompt => &self.system_prompt,
        }
    }

    fn draft_field_value(&self, field: SettingsField) -> &str {
        if self.editing && self.selected_field() == Some(field) {
            &self.edit_buffer
        } else {
            self.field_value(field)
        }
    }

    pub(crate) fn resolved_endpoint_preview(&self) -> ResolvedEndpointPreview {
        let model = self.draft_field_value(SettingsField::Model).trim();
        let base_url = self.draft_field_value(SettingsField::BaseUrl).trim();
        let base_url = (!base_url.is_empty()).then_some(base_url);

        match smolllm::resolve_endpoints(model, base_url) {
            Ok(endpoints) => {
                let additional = endpoints.len().saturating_sub(1);
                let first = endpoints
                    .first()
                    .expect("valid model list resolved without an endpoint");
                let suffix = if additional == 0 {
                    String::new()
                } else {
                    format!(" (+{additional} more)")
                };
                ResolvedEndpointPreview::Ready(format!("POST {}{suffix}", first.url))
            }
            Err(error) => ResolvedEndpointPreview::Error(error.to_string()),
        }
    }

    fn field_mut(&mut self, field: SettingsField) -> &mut String {
        match field {
            SettingsField::Model => &mut self.model,
            SettingsField::ApiKey => &mut self.api_key,
            SettingsField::BaseUrl => &mut self.base_url,
            SettingsField::MaxComments => &mut self.max_comments,
            SettingsField::IncludeArticle => &mut self.include_article,
            SettingsField::MaxArticleChars => &mut self.max_article_chars,
            SettingsField::SystemPrompt => &mut self.system_prompt,
        }
    }

    pub fn start_editing(&mut self) {
        let field = self
            .selected_field()
            .expect("connection-test row cannot enter text editing");
        self.editing = true;
        self.edit_buffer = self.field_value(field).to_string();
        self.edit_cursor = self.edit_buffer.chars().count();
    }

    pub fn confirm_edit(&mut self) {
        let val = self.edit_buffer.clone();
        let field = self
            .selected_field()
            .expect("settings edit completed without an editable field");
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

    #[test]
    fn resolved_endpoint_preview_uses_uncommitted_model_edit_buffer() {
        let mut popup = SettingsPopup::from_summarize(None, None);
        popup.model = "custom/saved".to_string();
        popup.base_url = "https://gateway.example".to_string();
        popup.cursor = 0;
        popup.start_editing();
        popup.edit_buffer = "custom/draft".to_string();

        let preview = popup.resolved_endpoint_preview();

        assert_eq!(
            preview,
            ResolvedEndpointPreview::Ready(
                "POST https://gateway.example/v1/chat/completions".to_string()
            )
        );
    }

    #[test]
    fn resolved_endpoint_preview_uses_uncommitted_base_url_edit_buffer() {
        let mut popup = SettingsPopup::from_summarize(None, None);
        popup.model = "custom/model".to_string();
        popup.base_url = "https://saved.example".to_string();
        popup.cursor = 2;
        popup.start_editing();
        popup.edit_buffer = "https://draft.example/custom#".to_string();

        let preview = popup.resolved_endpoint_preview();

        assert_eq!(
            preview,
            ResolvedEndpointPreview::Ready("POST https://draft.example/custom".to_string())
        );
    }

    #[test]
    fn resolved_endpoint_preview_keeps_actionable_resolution_error() {
        let mut popup = SettingsPopup::from_summarize(None, None);
        popup.model = "hntui-issue-25-unknown/model".to_string();

        let preview = popup.resolved_endpoint_preview();

        assert_eq!(
            preview,
            ResolvedEndpointPreview::Error(
                "missing base URL for provider 'hntui-issue-25-unknown'. \
                 Pass base_url or set HNTUI_ISSUE_25_UNKNOWN_BASE_URL"
                    .to_string()
            )
        );
    }

    #[test]
    fn resolved_endpoint_preview_summarizes_additional_models() {
        let mut popup = SettingsPopup::from_summarize(None, None);
        popup.model = "custom/first, custom/second, custom/third".to_string();
        popup.base_url = "https://gateway.example".to_string();

        let preview = popup.resolved_endpoint_preview();

        assert_eq!(
            preview,
            ResolvedEndpointPreview::Ready(
                "POST https://gateway.example/v1/chat/completions (+2 more)".to_string()
            )
        );
    }

    #[test]
    fn connection_test_is_the_eighth_non_editable_settings_row() {
        let mut popup = SettingsPopup::from_summarize(None, None);

        assert_eq!(SettingsPopup::rows().len(), 8);
        popup.cursor = 7;
        assert_eq!(popup.selected_row(), SettingsRow::TestConnection);
        assert_eq!(popup.selected_field(), None);
        assert_eq!(popup.connection_test, ConnectionTestState::Idle);
    }
}
