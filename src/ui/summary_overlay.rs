use crate::api::types::Story;
use crate::summarizer::SummaryEvent;
use crate::ui::{markdown, theme};
#[cfg(not(target_os = "android"))]
use anyhow::Context;
use anyhow::Result;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};
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
    scroll_offset: usize,
    comment_count: usize,
    viewport_width: u16,
    viewport_height: usize,
    wrapped_line_count: usize,
    reasoning: String,
    content_started: bool,
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
        self.scroll_offset = 0;
        self.comment_count = comment_count;
        self.reasoning.clear();
        self.content_started = false;
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
                        self.scroll_offset = 0;
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
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
        self.clamp_scroll();
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
        self.clamp_scroll();
    }

    pub fn go_top(&mut self) {
        self.scroll_offset = 0;
        self.clamp_scroll();
    }

    pub fn go_bottom(&mut self) {
        self.scroll_offset = self.max_scroll_offset();
    }

    pub fn set_viewport(&mut self, width: u16, height: u16) {
        self.viewport_width = width;
        self.viewport_height = usize::from(height);
        self.reflow();
    }

    pub fn page_scroll_amount(&self) -> usize {
        self.viewport_height.saturating_sub(2).max(1)
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    pub fn wrapped_line_count(&self) -> usize {
        self.wrapped_line_count
    }

    fn max_scroll_offset(&self) -> usize {
        self.wrapped_line_count()
            .saturating_sub(self.viewport_height)
    }

    fn content_overflows_viewport(&self) -> bool {
        self.wrapped_line_count() > self.viewport_height
    }

    fn clamp_scroll(&mut self) {
        self.scroll_offset = if self.is_reasoning_phase() {
            self.max_scroll_offset()
        } else {
            self.scroll_offset.min(self.max_scroll_offset())
        };
    }

    fn is_reasoning_phase(&self) -> bool {
        matches!(self.state, SummaryState::Loading | SummaryState::Streaming)
            && !self.content_started
    }

    fn reflow(&mut self) {
        self.wrapped_line_count = self.content_paragraph(' ').line_count(self.viewport_width);
        self.clamp_scroll();
    }

    fn render_scroll_offset(&self) -> u16 {
        self.scroll_offset()
            .try_into()
            .expect("clamped summary scroll offset exceeds ratatui's u16 limit")
    }

    pub fn state(&self) -> SummaryState {
        self.state
    }

    pub fn is_visible(&self) -> bool {
        self.state != SummaryState::Idle
    }

    fn copy_text(&self) -> String {
        let hn_link = format!("https://news.ycombinator.com/item?id={}", self.story_id);
        let mut output = String::from("---\n");
        output.push_str(&format!(
            "title: \"{}\"\n",
            self.story_title.replace('"', "\\\"")
        ));
        if let Some(url) = &self.story_url {
            output.push_str(&format!("source: {url}\n"));
        }
        output.push_str(&format!("hn: {hn_link}\n"));
        output.push_str(&format!("score: {}\n", self.story_score));
        output.push_str(&format!("author: {}\n", self.story_author));
        output.push_str(&format!("comments: {}\n", self.comment_count));
        output.push_str(&format!("model: {}\n", self.model_name));
        let date = chrono::DateTime::from_timestamp(self.story_time, 0)
            .map(|date| date.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        output.push_str(&format!("date: {date}\n"));
        output.push_str("---\n\n");
        output.push_str(&self.summary);
        output
    }

    fn content_lines(&self, spinner: char) -> Vec<Line<'static>> {
        match self.state {
            SummaryState::Loading if self.reasoning.is_empty() => {
                vec![Line::from(Span::styled(
                    format!("Waiting for LLM response {spinner}"),
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
            SummaryState::Idle => Vec::new(),
        }
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
    let Some(popup) = popup_rect(frame.area()) else {
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
    let inner = block.inner(popup);
    let layout = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(inner);
    let content_area = layout[0];
    let hint_area = layout[1];
    frame.render_widget(Clear, popup);
    frame.render_widget(block.style(theme::POPUP), popup);
    frame.render_widget(
        overlay
            .content_paragraph(spinner)
            .scroll((overlay.render_scroll_offset(), 0))
            .style(theme::POPUP),
        content_area,
    );
    if overlay.content_overflows_viewport() {
        // ScrollbarState counts reachable positions. `max + 1` keeps its thumb
        // aligned with the viewport: top at offset 0, bottom at max offset.
        let mut scrollbar_state =
            ScrollbarState::new(overlay.max_scroll_offset().saturating_add(1))
                .position(overlay.scroll_offset())
                .viewport_content_length(overlay.viewport_height);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            content_area,
            &mut scrollbar_state,
        );
    }
    let show_copied = overlay
        .copied_flash
        .is_some_and(|timestamp| timestamp.elapsed().as_secs() < 2);
    let hint = if show_copied {
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
    frame.render_widget(Paragraph::new(hint).style(theme::POPUP), hint_area);
}

pub(crate) fn popup_rect(area: Rect) -> Option<Rect> {
    if area.width < 12 || area.height < 8 {
        return None;
    }
    Some(super::centered(
        area,
        (area.width * 4 / 5).max(30),
        (area.height * 4 / 5).max(10),
    ))
}

pub(crate) fn content_area(area: Rect) -> Option<Rect> {
    let popup = popup_rect(area)?;
    let inner = Block::default().borders(Borders::ALL).inner(popup);
    let layout = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(inner);
    Some(layout[0])
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
