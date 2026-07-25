use crate::api::types::Story;
use crate::article::Article;
use crate::ui::{clamped_scroll::ClampedScroll, markdown, overlay, theme};
#[cfg(not(target_os = "android"))]
use anyhow::Context;
use anyhow::Result;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;
use std::time::Instant;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ArticleState {
    #[default]
    Idle,
    Loading,
    Done,
    Error,
}

#[derive(Default)]
pub struct ArticleOverlay {
    state: ArticleState,
    content: String,
    article_title: Option<String>,
    error: Option<String>,
    scroll: ClampedScroll,
    viewport_width: u16,
    /// Elapsed time is the only progress signal available across the process
    /// boundary — localwebrs reports nothing until it is done.
    started_at: Option<Instant>,
    copied_flash: Option<Instant>,
    story_title: String,
    story_url: Option<String>,
    story_id: u64,
    story_time: i64,
}

impl ArticleOverlay {
    /// Open in Loading for a story whose Article is still being fetched.
    pub fn begin(&mut self, story: &Story) {
        self.reset_for(story);
        self.state = ArticleState::Loading;
        self.started_at = Some(Instant::now());
        self.reflow();
    }

    /// Open with an Article already in hand (memory hit or self-post body).
    pub fn show(&mut self, story: &Story, article: Article) {
        self.reset_for(story);
        self.finish(article);
    }

    /// Open straight into the error state — nothing to fetch, or a fetch that
    /// failed before the overlay existed.
    pub fn show_error(&mut self, story: &Story, message: String) {
        self.reset_for(story);
        self.fail(message);
    }

    pub fn finish(&mut self, article: Article) {
        self.state = ArticleState::Done;
        self.article_title = article.title;
        self.content = article.content;
        self.error = None;
        self.started_at = None;
        self.scroll.go_top();
        self.reflow();
    }

    pub fn fail(&mut self, message: String) {
        self.state = ArticleState::Error;
        self.error = Some(message);
        self.started_at = None;
        self.scroll.go_top();
        self.reflow();
    }

    pub fn dismiss(&mut self) {
        *self = Self::default();
    }

    fn reset_for(&mut self, story: &Story) {
        *self = Self {
            story_title: story.title.clone(),
            story_url: story.url.clone(),
            story_id: story.id,
            story_time: story.time,
            ..Self::default()
        };
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll.scroll_down(amount);
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll.scroll_up(amount);
    }

    pub fn go_top(&mut self) {
        self.scroll.go_top();
    }

    pub fn go_bottom(&mut self) {
        self.scroll.go_bottom();
    }

    pub fn set_viewport(&mut self, width: u16, height: u16) {
        self.viewport_width = width;
        self.scroll.set_viewport_height(usize::from(height));
        self.reflow();
    }

    pub fn page_scroll_amount(&self) -> usize {
        self.scroll.page_amount()
    }

    #[cfg(test)]
    pub fn scroll_offset(&self) -> usize {
        self.scroll.offset()
    }

    #[cfg(test)]
    pub fn wrapped_line_count(&self) -> usize {
        self.scroll.content_height()
    }

    #[cfg(test)]
    pub fn state(&self) -> ArticleState {
        self.state
    }

    pub fn is_visible(&self) -> bool {
        self.state != ArticleState::Idle
    }

    /// The story this overlay is showing, so a settled fetch can be matched
    /// against what the user is actually looking at.
    pub fn story_id(&self) -> u64 {
        self.story_id
    }

    pub fn story_url(&self) -> Option<&str> {
        self.story_url.as_deref()
    }

    fn reflow(&mut self) {
        let wrapped_line_count = self.content_paragraph(' ').line_count(self.viewport_width);
        self.scroll.set_content_height(wrapped_line_count);
    }

    fn render_scroll_offset(&self) -> u16 {
        self.scroll.render_offset()
    }

    fn elapsed_secs(&self) -> u64 {
        self.started_at
            .map(|started| started.elapsed().as_secs())
            .unwrap_or(0)
    }

    /// Symmetric with the summary copy: YAML front matter, then the markdown.
    fn copy_text(&self) -> String {
        let mut output = String::from("---\n");
        output.push_str(&overlay::front_matter_title(
            self.article_title.as_deref().unwrap_or(&self.story_title),
        ));
        if let Some(url) = &self.story_url {
            output.push_str(&format!("source: {url}\n"));
        }
        output.push_str(&format!("hn: {}\n", overlay::hn_url(self.story_id)));
        output.push_str(&overlay::front_matter_date(self.story_time));
        output.push_str("---\n\n");
        output.push_str(&self.content);
        output
    }

    fn content_lines(&self, spinner: char) -> Vec<Line<'static>> {
        match self.state {
            ArticleState::Loading => vec![Line::from(Span::styled(
                format!(
                    "fetching article… {}s {spinner} (Esc to cancel)",
                    self.elapsed_secs()
                ),
                theme::HINT,
            ))],
            ArticleState::Done => markdown::render_markdown(&self.content),
            ArticleState::Error => vec![Line::from(Span::styled(
                self.error.as_deref().unwrap_or("Unknown error").to_string(),
                theme::ERROR,
            ))],
            ArticleState::Idle => Vec::new(),
        }
    }

    fn content_paragraph(&self, spinner: char) -> Paragraph<'static> {
        Paragraph::new(self.content_lines(spinner)).wrap(Wrap { trim: false })
    }

    #[cfg(not(target_os = "android"))]
    pub fn copy_article(&mut self) -> Result<()> {
        anyhow::ensure!(!self.content.is_empty(), "article is empty");
        let mut clipboard = arboard::Clipboard::new().context("open clipboard")?;
        clipboard
            .set_text(self.copy_text())
            .context("copy article")?;
        self.copied_flash = Some(Instant::now());
        Ok(())
    }

    #[cfg(target_os = "android")]
    pub fn copy_article(&mut self) -> Result<()> {
        anyhow::bail!("clipboard unavailable on Android")
    }
}

pub fn render(frame: &mut Frame, overlay: &ArticleOverlay, spinner: char) {
    if !overlay.is_visible() {
        return;
    }
    let Some(areas) = overlay::areas(frame.area()) else {
        return;
    };

    let source = overlay
        .story_url
        .as_deref()
        .and_then(super::domain_from_url)
        .unwrap_or_else(|| "self".to_string());
    let title = match overlay.state {
        ArticleState::Loading => format!(" Article {spinner} ({source}) "),
        ArticleState::Done => format!(" {} ({source}) ", overlay.story_title),
        ArticleState::Error => format!(" {} — no article ({source}) ", overlay.story_title),
        ArticleState::Idle => return,
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, theme::HEADER_ACCENT));
    frame.render_widget(Clear, areas.popup);
    frame.render_widget(block.style(theme::POPUP), areas.popup);
    frame.render_widget(
        overlay
            .content_paragraph(spinner)
            .scroll((overlay.render_scroll_offset(), 0))
            .style(theme::POPUP),
        areas.content,
    );
    overlay::render_scrollbar(frame, areas.scrollbar, &overlay.scroll);

    let hint = if overlay::copied_recently(overlay.copied_flash) {
        Line::from(Span::styled("Copied!", theme::SUCCESS))
    } else {
        let text = match overlay.state {
            ArticleState::Done => "j/k: scroll  c: copy  o: browser  q/Esc: close",
            ArticleState::Error => "o: browser  q/Esc: close",
            _ => "q/Esc: cancel",
        };
        Line::from(Span::styled(text, theme::HINT))
    };
    frame.render_widget(Paragraph::new(hint).style(theme::POPUP), areas.hint);
}

pub(crate) fn popup_rect(area: Rect) -> Option<Rect> {
    overlay::popup_rect(area)
}

pub(crate) fn content_area(area: Rect) -> Option<Rect> {
    Some(overlay::areas(area)?.content)
}

#[cfg(test)]
mod tests;
