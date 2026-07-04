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
    Started { model: String },
    Chunk { content: String, reasoning: String },
    Complete,
    Error { message: String },
}
