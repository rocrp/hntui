pub mod config;
pub mod llm;
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
    SummarizeChunk { delta: String },
    SummarizeComplete,
    SummarizeError { message: String },
}
