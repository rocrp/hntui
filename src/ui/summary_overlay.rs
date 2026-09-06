use crate::api::types::Story;
use crate::summarizer::{SummaryEvent, SummaryStats};
use crate::ui::{clamped_scroll::ClampedScroll, markdown, overlay, theme};
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
    /// The ResolvedModel, when the server named one that differs from what was
    /// requested.
    resolved_model: Option<String>,
    /// What the finished call cost. None until it finishes.
    stats: Option<SummaryStats>,
    /// Whether the summary was grounded in the Article.
    article_included: bool,
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
        self.resolved_model = None;
        self.stats = None;
        self.article_included = false;
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
    /// Whether the Article made it into the prompt.
    pub fn set_article_included(&mut self, included: bool) {
        self.article_included = included;
    }

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
            SummaryEvent::Started {
                model,
                resolved_model,
            } => {
                // Only worth showing when it says something the request did not.
                self.resolved_model = resolved_model.filter(|resolved| *resolved != model);
                self.model_name = model;
            }
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
            SummaryEvent::Complete { stats } => {
                self.stats = stats;
                self.state = SummaryState::Done;
            }
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

    /// Whether the answer was cut short. Only known once the call finished.
    pub(crate) fn truncated(&self) -> bool {
        self.stats.is_some_and(|stats| stats.truncated)
    }

    /// The one-line cost summary shown beside the key hints once a summary is
    /// done: what went in, how long it took, and what it spent.
    pub(crate) fn stats_line(&self) -> Option<String> {
        let stats = self.stats?;
        let article = if self.article_included { "✓" } else { "✗" };
        let mut parts = vec![
            format!("{} comments", self.comment_count),
            format!("article {article}"),
            format_duration(stats.duration),
        ];
        if let Some(ttft) = stats.ttft {
            parts.push(format!("ttft {}", format_duration(ttft)));
        }
        // A chars/4 guess is not a measurement; showing it would invite the
        // reader to trust a number nobody counted.
        if !stats.estimated {
            parts.push(format!(
                "{}→{} tok",
                format_tokens(stats.input_tokens),
                format_tokens(stats.output_tokens)
            ));
        }
        Some(parts.join(" · "))
    }

    /// How the model reads in the title: `requested → resolved` when a proxy or
    /// alias resolved it to something else, the requested spec alone otherwise.
    pub(crate) fn model_label(&self) -> String {
        match &self.resolved_model {
            Some(resolved) => format!("{} → {resolved}", self.model_name),
            None => self.model_name.clone(),
        }
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
        if let Some(resolved) = &self.resolved_model {
            output.push_str(&format!("resolved_model: {resolved}\n"));
        }
        if let Some(stats) = self.stats {
            output.push_str(&format!("duration: {}\n", format_duration(stats.duration)));
            if stats.truncated {
                output.push_str("truncated: true\n");
            }
        }
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
            // Whatever streamed before the failure is still worth reading, so
            // it stays above the error rather than being thrown away with it.
            SummaryState::Error => {
                let mut lines = markdown::render_markdown(&self.summary);
                if !lines.is_empty() {
                    lines.push(Line::raw(""));
                }
                lines.push(Line::from(Span::styled(
                    self.error.as_deref().unwrap_or("Unknown error").to_string(),
                    theme::ERROR,
                )));
                lines
            }
            SummaryState::Idle => return Vec::new(),
        };

        let mut notices: Vec<Line<'static>> = Vec::new();
        if let Some(notice) = &self.article_notice {
            notices.push(Line::from(Span::styled(
                format!("⚠ article unavailable ({notice}) — comments only"),
                theme::WARN,
            )));
        }
        if self.truncated() {
            notices.push(Line::from(Span::styled(
                "⚠ output truncated".to_string(),
                theme::WARN,
            )));
        }
        if notices.is_empty() {
            return body;
        }
        notices.push(Line::raw(""));
        notices.extend(body);
        notices
    }

    fn content_paragraph(&self, spinner: char) -> Paragraph<'static> {
        Paragraph::new(self.content_lines(spinner)).wrap(Wrap { trim: false })
    }

    /// The document `c` copies, or why there is nothing to copy.
    pub(crate) fn copyable_text(&self) -> Result<String> {
        anyhow::ensure!(!self.summary.is_empty(), "summary is empty");
        Ok(self.copy_text())
    }

    pub(crate) fn mark_copied(&mut self) {
        self.copied_flash = Some(Instant::now());
    }
}

/// Sub-second times read better in milliseconds; anything longer in seconds.
fn format_duration(duration: std::time::Duration) -> String {
    if duration < std::time::Duration::from_secs(1) {
        format!("{}ms", duration.as_millis())
    } else {
        format!("{:.1}s", duration.as_secs_f64())
    }
}

/// Token counts are read at a glance, so thousands are abbreviated.
fn format_tokens(tokens: usize) -> String {
    if tokens < 1_000 {
        return tokens.to_string();
    }
    format!("{:.1}k", tokens as f64 / 1_000.0)
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
        format!(" ({})", overlay.model_label())
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
    // The stats share the hint row rather than claiming a line of their own:
    // the overlay geometry is shared with the Article overlay, and the hints
    // leave most of the row empty anyway.
    let stats = overlay.stats_line();
    let hint_area = match &stats {
        Some(stats) => {
            let width = u16::try_from(stats.chars().count() + 2).unwrap_or(u16::MAX);
            let [hint_area, stats_area] = ratatui::layout::Layout::horizontal([
                ratatui::layout::Constraint::Min(0),
                ratatui::layout::Constraint::Length(width.min(areas.hint.width)),
            ])
            .areas(areas.hint);
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(stats.clone(), theme::META)))
                    .alignment(ratatui::layout::Alignment::Right)
                    .style(theme::POPUP),
                stats_area,
            );
            hint_area
        }
        None => areas.hint,
    };
    frame.render_widget(Paragraph::new(hint).style(theme::POPUP), hint_area);
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
