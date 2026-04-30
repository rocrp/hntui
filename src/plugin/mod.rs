pub mod config;
pub mod summarize;

use crate::api::types::{Comment, Story};
use crate::app::AppEvent;
use tokio::sync::mpsc;

pub struct PluginContext<'a> {
    pub current_story: Option<&'a Story>,
    pub comment_list: &'a [Comment],
    pub tx: mpsc::UnboundedSender<AppEvent>,
}

#[derive(Debug)]
pub enum PluginEvent {
    SummarizeStarted { model: String },
    SummarizeChunk { content: String, reasoning: String },
    SummarizeComplete,
    SummarizeError { message: String },
}
