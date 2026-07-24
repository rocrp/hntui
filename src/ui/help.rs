use crate::app::{App, View};
use crate::ui::theme;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

#[derive(Default)]
pub struct HelpOverlay {
    scroll_offset: usize,
    viewport_height: usize,
    wrapped_line_count: usize,
}

impl HelpOverlay {
    pub fn open(&mut self) {
        self.scroll_offset = 0;
    }

    pub fn set_frame(&mut self, area: Rect, active: View) {
        let Some(popup) = popup_rect(area, active) else {
            self.viewport_height = 0;
            self.wrapped_line_count = 0;
            self.clamp_scroll();
            return;
        };
        let inner = Block::default().borders(Borders::ALL).inner(popup);
        self.viewport_height = usize::from(inner.height);
        self.wrapped_line_count = content_paragraph(active).line_count(inner.width);
        self.clamp_scroll();
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
        self.clamp_scroll();
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
        self.clamp_scroll();
    }

    pub fn page_scroll_amount(&self) -> usize {
        self.viewport_height.saturating_sub(2).max(1)
    }

    #[cfg(test)]
    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    #[cfg(test)]
    fn wrapped_line_count(&self) -> usize {
        self.wrapped_line_count
    }

    fn max_scroll_offset(&self) -> usize {
        self.wrapped_line_count.saturating_sub(self.viewport_height)
    }

    fn clamp_scroll(&mut self) {
        self.scroll_offset = self.scroll_offset.min(self.max_scroll_offset());
    }

    fn render_scroll_offset(&self) -> u16 {
        self.scroll_offset
            .try_into()
            .expect("clamped help scroll offset exceeds ratatui's u16 limit")
    }
}

fn section_title(name: &str, active: bool) -> Line<'static> {
    Line::from(Span::styled(
        name.to_string(),
        theme::section_heading(active),
    ))
}

fn kv(keys: &str, desc: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {keys}"), theme::KEY),
        Span::styled(format!(": {desc}"), theme::LABEL),
    ])
}

fn content_lines(active: View) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled("Shortcuts", theme::HEADER)),
        Line::from(Span::styled("Press ? or Esc to close.", theme::HINT)),
        Line::raw(""),
    ];

    let stories_active = active == View::Stories;
    lines.push(section_title("Stories", stories_active));
    lines.push(kv("j/k, ↓/↑", "move"));
    lines.push(kv("gg, G", "top / bottom"));
    lines.push(kv("Ctrl+d / Ctrl+u", "page down / up"));
    lines.push(kv("Enter / Space / l / →", "open comments"));
    lines.push(kv("o", "open source link (browser)"));
    lines.push(kv("O", "open comments page (browser)"));
    lines.push(kv("/", "search stories"));
    lines.push(kv("f", "switch feed"));
    lines.push(kv("F", "filter by title"));
    lines.push(kv("r", "refresh"));
    lines.push(kv(",", "settings"));
    lines.push(kv("q / Esc", "quit"));
    lines.push(Line::raw(""));

    let comments_active = active == View::Comments;
    lines.push(section_title("Comments", comments_active));
    lines.push(kv("j/k, ↓/↑", "move"));
    lines.push(kv("gg, G", "top / bottom"));
    lines.push(kv("Ctrl+d / Ctrl+u", "page down / up"));
    lines.push(kv("h / ←", "collapse thread"));
    lines.push(kv("l / →", "expand thread (loads children)"));
    lines.push(kv("Enter / c", "toggle collapse/expand"));
    lines.push(kv("y", "copy selected comment to clipboard"));
    lines.push(kv("o", "open comments page (browser)"));
    lines.push(kv("O", "open source link (browser)"));
    lines.push(kv("s", "summarize (AI)"));
    lines.push(kv("r", "refresh"));
    lines.push(kv(",", "settings"));
    lines.push(kv("q / Esc", "back"));
    lines.push(Line::raw(""));

    lines.push(section_title("Summary", false));
    lines.push(kv("gg, G", "top / bottom"));
    lines
}

fn content_paragraph(active: View) -> Paragraph<'static> {
    Paragraph::new(Text::from(content_lines(active))).wrap(Wrap { trim: true })
}

pub(crate) fn popup_rect(area: Rect, active: View) -> Option<Rect> {
    if area.width < 10 || area.height < 6 {
        return None;
    }
    let desired_width = area.width.min(76);
    let inner_width = desired_width.saturating_sub(2);
    let wrapped_height = content_paragraph(active).line_count(inner_width);
    let desired_height = wrapped_height
        .saturating_add(2)
        .min(usize::from(area.height))
        .try_into()
        .expect("help popup height is capped to the terminal");
    Some(super::centered(area, desired_width, desired_height))
}

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let Some(popup) = popup_rect(area, app.view) else {
        return;
    };

    let active = app.view;
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled("?", theme::HEADER));
    let paragraph = content_paragraph(active)
        .scroll((app.help_overlay.render_scroll_offset(), 0))
        .block(block)
        .style(theme::POPUP);
    frame.render_widget(paragraph, popup);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    #[test]
    fn scrolling_stops_when_the_last_help_line_reaches_the_viewport_bottom() {
        let mut overlay = HelpOverlay::default();
        overlay.set_frame(Rect::new(0, 0, 80, 10), View::Stories);

        overlay.scroll_down(usize::MAX);
        let bottom = overlay.scroll_offset();
        overlay.scroll_down(1);

        assert_eq!(bottom + 8, overlay.wrapped_line_count());
        assert_eq!(overlay.scroll_offset(), bottom);
    }

    #[test]
    fn help_that_fits_in_a_tall_terminal_does_not_scroll() {
        let mut overlay = HelpOverlay::default();
        overlay.set_frame(Rect::new(0, 0, 80, 100), View::Stories);

        overlay.scroll_down(usize::MAX);

        assert_eq!(overlay.scroll_offset(), 0);
    }

    #[test]
    fn narrow_terminal_sizes_the_popup_for_wrapped_help_lines() {
        let wide = popup_rect(Rect::new(0, 0, 80, 100), View::Stories).unwrap();
        let narrow = popup_rect(Rect::new(0, 0, 30, 100), View::Stories).unwrap();

        assert!(
            narrow.height > wide.height,
            "narrow help should grow from {wide:?}, got {narrow:?}"
        );
    }
}
