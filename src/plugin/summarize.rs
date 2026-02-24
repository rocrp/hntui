use crate::plugin::config::SummarizeConfig;
use crate::plugin::llm::{stream_chat_completion, ChatMessage};
use crate::plugin::{PluginContext, PluginEvent};
use crate::ui::comment_view::hn_html_to_plain;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummarizeState {
    Idle,
    Loading,
    Streaming,
    Done,
    Error,
}

pub struct SummarizePlugin {
    config: Option<SummarizeConfig>,
    state: SummarizeState,
    pub summary_text: String,
    pub error: Option<String>,
    pub scroll_offset: usize,
    pub comment_count: usize,
    /// Set during render: visible content height in rows (for page scroll)
    pub content_height: usize,
    /// LLM model name for display in overlay title
    pub model_name: String,
    http: reqwest::Client,
}

impl SummarizePlugin {
    pub fn new(config: Option<SummarizeConfig>, http: reqwest::Client) -> Self {
        Self {
            config,
            state: SummarizeState::Idle,
            summary_text: String::new(),
            error: None,
            scroll_offset: 0,
            comment_count: 0,
            content_height: 0,
            model_name: String::new(),
            http,
        }
    }

    pub fn state(&self) -> SummarizeState {
        self.state
    }

    pub fn is_overlay_visible(&self) -> bool {
        self.state != SummarizeState::Idle
    }

    pub fn dismiss(&mut self) {
        self.state = SummarizeState::Idle;
        self.summary_text.clear();
        self.error = None;
        self.scroll_offset = 0;
        self.comment_count = 0;
        self.model_name.clear();
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }

    pub fn start(&mut self, ctx: &PluginContext) {
        let Some(config) = &self.config else {
            self.state = SummarizeState::Error;
            self.error = Some(
                "LLM not configured. Create plugin-config.toml or set HNTUI_LLM_API_KEY"
                    .to_string(),
            );
            return;
        };

        let Some(api_key) = config.resolve_api_key() else {
            self.state = SummarizeState::Error;
            self.error = Some(
                "API key not set. Set HNTUI_LLM_API_KEY env var or api_key in plugin-config.toml"
                    .to_string(),
            );
            return;
        };

        let Some(story) = ctx.current_story else {
            self.state = SummarizeState::Error;
            self.error = Some("No story selected".to_string());
            return;
        };

        if ctx.comment_list.is_empty() {
            self.state = SummarizeState::Error;
            self.error = Some("No comments to summarize".to_string());
            return;
        }

        self.state = SummarizeState::Loading;
        self.summary_text.clear();
        self.error = None;
        self.scroll_offset = 0;
        self.comment_count = ctx.comment_list.len();
        self.model_name = config.model.clone();

        let prompt = build_prompt(story, ctx.comment_list, config.max_comments);
        let messages = vec![
            ChatMessage {
                role: "system",
                content: config.system_prompt.clone(),
            },
            ChatMessage {
                role: "user",
                content: prompt,
            },
        ];

        let http = self.http.clone();
        let api_url = config.api_url.clone();
        let model = config.model.clone();
        let tx = ctx.tx.clone();

        tokio::spawn(async move {
            stream_chat_completion(&http, &api_url, &api_key, &model, messages, tx).await;
        });
    }

    pub fn handle_event(&mut self, event: PluginEvent) {
        match event {
            PluginEvent::SummarizeChunk { delta } => {
                if self.state == SummarizeState::Loading {
                    self.state = SummarizeState::Streaming;
                }
                self.summary_text.push_str(&delta);
            }
            PluginEvent::SummarizeComplete => {
                self.state = SummarizeState::Done;
            }
            PluginEvent::SummarizeError { message } => {
                self.state = SummarizeState::Error;
                self.error = Some(message);
            }
        }
    }
}

fn build_prompt(
    story: &crate::api::types::Story,
    comments: &[crate::api::types::Comment],
    max_comments: usize,
) -> String {
    let mut prompt = format!("# {}\n\n", story.title);
    let mut count = 0;
    for comment in comments {
        if count >= max_comments {
            break;
        }
        if comment.deleted || comment.dead {
            continue;
        }
        let author = comment.by.as_deref().unwrap_or("[anon]");
        let indent = "  ".repeat(comment.depth);
        let text = hn_html_to_plain(&comment.text);
        prompt.push_str(&format!("{indent}{author}: {text}\n\n"));
        count += 1;
    }
    prompt
}
