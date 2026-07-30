use crate::api::types::Story;
use crate::summarizer::SummaryEvent;
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
pub enum SummaryState {
    #[default]
    Idle,
    Loading,
    Streaming,
    Done,
    Error,
}

#[derive(Default)]
pub struct SummaryOverlay {
    state: SummaryState,
    summary: String,
    error: Option<String>,
    scroll: ClampedScroll,
    comment_count: usize,
    viewport_width: u16,
    reasoning: String,
    content_started: bool,
    /// What the summary is blocked on before the LLM call starts.
    waiting_for: Option<String>,
    /// Set when the summary went ahead without an Article it should have had.
    article_notice: Option<String>,
    model_name: String,
    copied_flash: Option<Instant>,
    story_title: String,
    story_url: Option<String>,
    story_id: u64,
    story_score: i64,
    story_author: String,
    story_time: i64,
}

impl SummaryOverlay {
    pub fn begin(&mut self, story: &Story, comment_count: usize) {
        self.state = SummaryState::Loading;
        self.summary.clear();
        self.error = None;
        self.scroll.go_top();
        self.comment_count = comment_count;
        self.reasoning.clear();
        self.content_started = false;
        self.waiting_for = None;
        self.article_notice = None;
        self.model_name.clear();
        self.copied_flash = None;
        self.story_title = story.title.clone();
        self.story_url = story.url.clone();
        self.story_id = story.id;
        self.story_score = story.score;
        self.story_author = story.by.clone();
        self.story_time = story.time;
        self.reflow();
    }

    /// Comment count is only known once the discussion has loaded, which can
    /// be after the overlay is already up.
    pub fn set_comment_count(&mut self, comment_count: usize) {
        self.comment_count = comment_count;
    }

    /// Label the wait while the summary's inputs are still being gathered.
    pub fn set_waiting_for(&mut self, waiting_for: Option<String>) {
        self.waiting_for = waiting_for;
        self.reflow();
    }

    /// Announce that the summary is comments-only because the Article failed.
    pub fn set_article_notice(&mut self, notice: Option<String>) {
        self.article_notice = notice;
        self.reflow();
    }

    #[cfg(test)]
    pub fn article_notice(&self) -> Option<&str> {
        self.article_notice.as_deref()
    }

    #[cfg(test)]
    pub(crate) fn error_message(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn handle_event(&mut self, event: SummaryEvent) {
        match event {
            SummaryEvent::Started { model } => self.model_name = model,
            SummaryEvent::Chunk { content, reasoning } => {
                if !reasoning.is_empty() && !self.content_started {
                    self.reasoning.push_str(&reasoning);
                    if self.state == SummaryState::Loading {
                        self.state = SummaryState::Streaming;
                    }
                }
                if !content.is_empty() {
                    if !self.content_started {
                        self.content_started = true;
                        self.scroll.go_top();
                    }
                    self.summary.push_str(&content);
                    if self.state == SummaryState::Loading {
                        self.state = SummaryState::Streaming;
                    }
                }
            }
            SummaryEvent::Complete => self.state = SummaryState::Done,
        }
        self.reflow();
    }

    pub fn fail(&mut self, message: String) {
        self.state = SummaryState::Error;
        self.error = Some(message);
        self.reflow();
    }

    pub fn dismiss(&mut self) {
        *self = Self::default();
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll.scroll_down(amount);
        self.pin_reasoning_to_tail();
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll.scroll_up(amount);
        self.pin_reasoning_to_tail();
    }

    pub fn go_top(&mut self) {
        self.scroll.go_top();
        self.pin_reasoning_to_tail();
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

    fn pin_reasoning_to_tail(&mut self) {
        if self.is_reasoning_phase() {
            self.scroll.go_bottom();
        }
    }

    fn is_reasoning_phase(&self) -> bool {
        matches!(self.state, SummaryState::Loading | SummaryState::Streaming)
            && !self.content_started
    }

    fn reflow(&mut self) {
        let wrapped_line_count = self.content_paragraph(' ').line_count(self.viewport_width);
        self.scroll.set_content_height(wrapped_line_count);
        self.pin_reasoning_to_tail();
    }

    fn render_scroll_offset(&self) -> u16 {
        self.scroll.render_offset()
    }

    pub fn state(&self) -> SummaryState {
        self.state
    }

    pub fn is_visible(&self) -> bool {
        self.state != SummaryState::Idle
    }

    fn copy_text(&self) -> String {
        let mut output = String::from("---\n");
        output.push_str(&overlay::front_matter_title(&self.story_title));
        if let Some(url) = &self.story_url {
            output.push_str(&format!("source: {url}\n"));
        }
        output.push_str(&format!("hn: {}\n", overlay::hn_url(self.story_id)));
        output.push_str(&format!("score: {}\n", self.story_score));
        output.push_str(&format!("author: {}\n", self.story_author));
        output.push_str(&format!("comments: {}\n", self.comment_count));
        output.push_str(&format!("model: {}\n", self.model_name));
        output.push_str(&overlay::front_matter_date(self.story_time));
        output.push_str("---\n\n");
        output.push_str(&self.summary);
        output
    }

    fn content_lines(&self, spinner: char) -> Vec<Line<'static>> {
        let body = match self.state {
            SummaryState::Loading if self.reasoning.is_empty() => {
                let label = self
                    .waiting_for
                    .as_deref()
                    .unwrap_or("Waiting for LLM response");
                vec![Line::from(Span::styled(
                    format!("{label} {spinner}"),
                    theme::HINT,
                ))]
            }
            SummaryState::Loading => reasoning_lines(&self.reasoning, spinner),
            SummaryState::Streaming if self.summary.is_empty() => {
                reasoning_lines(&self.reasoning, spinner)
            }
            SummaryState::Streaming => {
                let mut lines = markdown::render_markdown(&self.summary);
                lines.push(Line::from(Span::styled(spinner.to_string(), theme::HINT)));
                lines
            }
            SummaryState::Done => markdown::render_markdown(&self.summary),
            SummaryState::Error => vec![Line::from(Span::styled(
                self.error.as_deref().unwrap_or("Unknown error").to_string(),
                theme::ERROR,
            ))],
            SummaryState::Idle => return Vec::new(),
        };

        let Some(notice) = &self.article_notice else {
            return body;
        };
        let mut lines = vec![
            Line::from(Span::styled(
                format!("⚠ article unavailable ({notice}) — comments only"),
                theme::WARN,
            )),
            Line::raw(""),
        ];
        lines.extend(body);
        lines
    }

    fn content_paragraph(&self, spinner: char) -> Paragraph<'static> {
        Paragraph::new(self.content_lines(spinner)).wrap(Wrap { trim: false })
    }

    #[cfg(not(target_os = "android"))]
    pub fn copy_summary(&mut self) -> Result<()> {
        anyhow::ensure!(!self.summary.is_empty(), "summary is empty");
        let mut clipboard = arboard::Clipboard::new().context("open clipboard")?;
        clipboard
            .set_text(self.copy_text())
            .context("copy summary")?;
        self.copied_flash = Some(Instant::now());
        Ok(())
    }

    #[cfg(target_os = "android")]
    pub fn copy_summary(&mut self) -> Result<()> {
        anyhow::bail!("clipboard unavailable on Android")
    }
}

pub fn render(frame: &mut Frame, overlay: &SummaryOverlay, spinner: char) {
    if !overlay.is_visible() {
        return;
    }
    let Some(areas) = overlay::areas(frame.area()) else {
        return;
    };
    let model_tag = if overlay.model_name.is_empty() {
        String::new()
    } else {
        format!(" ({})", overlay.model_name)
    };
    let title = match overlay.state {
        SummaryState::Loading if overlay.reasoning.is_empty() => format!(
            " Summarizing {spinner} ({} comments){model_tag} ",
            overlay.comment_count
        ),
        SummaryState::Loading => format!(" Thinking {spinner}{model_tag} "),
        SummaryState::Streaming if overlay.summary.is_empty() => {
            format!(" Thinking {spinner}{model_tag} ")
        }
        SummaryState::Streaming => format!(" Summarizing {spinner}{model_tag} "),
        SummaryState::Done => format!(" Summary{model_tag} "),
        SummaryState::Error => " Summary Error ".to_string(),
        SummaryState::Idle => return,
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
            SummaryState::Done => "j/k: scroll  c: copy  q/Esc: close",
            SummaryState::Streaming => "j/k: scroll  c: copy  q/Esc: cancel",
            SummaryState::Error => "j/k: scroll  q/Esc: close",
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

fn reasoning_lines(buffer: &str, spinner: char) -> Vec<Line<'static>> {
    use ratatui::style::{Modifier, Style};
    let style = Style::default()
        .fg(theme::OVERLAY0)
        .add_modifier(Modifier::DIM | Modifier::ITALIC);
    let mut lines = vec![
        Line::from(Span::styled(format!("Thinking {spinner}"), theme::HINT)),
        Line::raw(""),
    ];
    lines.extend(
        buffer
            .lines()
            .map(|line| Line::from(Span::styled(line.to_string(), style))),
    );
    lines
}

#[cfg(test)]
mod render_tests;

#[cfg(test)]
mod tests;
