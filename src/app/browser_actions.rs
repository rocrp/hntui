use super::App;
use crate::api::Story;
use anyhow::{Context, Result};

impl App {
    pub(super) fn open_selected_story_in_browser(&self) -> Result<crate::browser::OpenOutcome> {
        let story = self.selected_story().context("no selected story")?;
        open_story(story)
    }

    pub(super) fn open_selected_story_comments_in_browser(
        &self,
    ) -> Result<crate::browser::OpenOutcome> {
        let story = self.selected_story().context("no selected story")?;
        open_story_comments(story)
    }

    pub(super) fn open_current_story_in_browser(&self) -> Result<crate::browser::OpenOutcome> {
        let story = self.current_story.as_ref().context("no current story")?;
        open_story(story)
    }

    pub(super) fn open_current_story_comments_in_browser(
        &self,
    ) -> Result<crate::browser::OpenOutcome> {
        let story = self.current_story.as_ref().context("no current story")?;
        open_story_comments(story)
    }
}

fn open_story(story: &Story) -> Result<crate::browser::OpenOutcome> {
    let url = story
        .url
        .clone()
        .unwrap_or_else(|| format!("https://news.ycombinator.com/item?id={}", story.id));
    crate::browser::open_url(&url)
}

fn open_story_comments(story: &Story) -> Result<crate::browser::OpenOutcome> {
    let url = format!("https://news.ycombinator.com/item?id={}", story.id);
    crate::browser::open_url(&url)
}
