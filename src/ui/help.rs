use crate::app::{App, View};
use crate::ui::{clamped_scroll::ClampedScroll, theme};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

/// Which overlay sits in front of the view when help is opened. Drives which
/// help section is highlighted as active.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum HelpFocus {
    #[default]
    View,
    Summary,
    Article,
}

#[derive(Default)]
pub struct HelpOverlay {
    visible: bool,
    scroll: ClampedScroll,
}

impl HelpOverlay {
    pub fn open(&mut self) {
        self.visible = true;
        self.scroll.go_top();
    }

    pub fn dismiss(&mut self) {
        self.visible = false;
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn set_frame(&mut self, area: Rect, active: View, focus: HelpFocus) {
        let Some(popup) = popup_rect(area, active, focus) else {
            self.scroll.set_extents(0, 0);
            return;
        };
        let inner = Block::default().borders(Borders::ALL).inner(popup);
        let wrapped_line_count = content_paragraph(active, focus).line_count(inner.width);
        self.scroll
            .set_extents(wrapped_line_count, usize::from(inner.height));
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll.scroll_down(amount);
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll.scroll_up(amount);
    }

    pub fn page_scroll_amount(&self) -> usize {
        self.scroll.page_amount()
    }

    #[cfg(test)]
    pub fn scroll_offset(&self) -> usize {
        self.scroll.offset()
    }

    #[cfg(test)]
    fn wrapped_line_count(&self) -> usize {
        self.scroll.content_height()
    }

    fn render_scroll_offset(&self) -> u16 {
        self.scroll.render_offset()
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

fn content_lines(active: View, focus: HelpFocus) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled("Shortcuts", theme::HEADER)),
        Line::from(Span::styled("Press ? or Esc to close.", theme::HINT)),
        Line::raw(""),
    ];

    let in_view = focus == HelpFocus::View;
    let stories_active = in_view && active == View::Stories;
    lines.push(section_title("Stories", stories_active));
    lines.push(kv("j/k, ↓/↑", "move"));
    lines.push(kv("gg, G", "top / bottom"));
    lines.push(kv("Ctrl+d / Ctrl+u", "page down / up"));
    lines.push(kv("Enter / Space / l / →", "open comments"));
    lines.push(kv("o", "open source link (browser)"));
    lines.push(kv("O", "open comments page (browser)"));
    lines.push(kv("s", "summarize (AI)"));
    lines.push(kv("v", "view article"));
    lines.push(kv("/", "search stories"));
    lines.push(kv("f", "switch feed"));
    lines.push(kv("F", "filter by title"));
    lines.push(kv("r", "refresh"));
    lines.push(kv(",", "settings"));
    lines.push(kv("q / Esc", "quit"));
    lines.push(Line::raw(""));

    let comments_active = in_view && active == View::Comments;
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
    lines.push(kv("v", "view article"));
    lines.push(kv("r", "refresh"));
    lines.push(kv(",", "settings"));
    lines.push(kv("q / Esc", "back"));
    lines.push(Line::raw(""));

    lines.push(section_title("Summary", focus == HelpFocus::Summary));
    lines.push(kv("gg, G", "top / bottom"));
    lines.push(Line::raw(""));

    lines.push(section_title("Article", focus == HelpFocus::Article));
    lines.push(kv("j/k, ↓/↑", "scroll"));
    lines.push(kv("gg, G", "top / bottom"));
    lines.push(kv("Ctrl+d / Ctrl+u", "page down / up"));
    lines.push(kv("Tab / Shift+Tab", "next / previous article link"));
    lines.push(kv("Enter", "open selected article link (browser)"));
    lines.push(kv("c", "copy article to clipboard"));
    lines.push(kv("o", "open Story's original URL (browser)"));
    lines.push(kv("q / Esc", "close (cancels a running fetch)"));
    lines
}

fn content_paragraph(active: View, focus: HelpFocus) -> Paragraph<'static> {
    Paragraph::new(Text::from(content_lines(active, focus))).wrap(Wrap { trim: true })
}

pub(crate) fn popup_rect(area: Rect, active: View, focus: HelpFocus) -> Option<Rect> {
    if area.width < 3 || area.height < 3 {
        return None;
    }
    let desired_width = area.width.min(76);
    let inner_width = desired_width.saturating_sub(2);
    let wrapped_height = content_paragraph(active, focus).line_count(inner_width);
    let desired_height = wrapped_height
        .saturating_add(2)
        .min(usize::from(area.height))
        .try_into()
        .expect("help popup height is capped to the terminal");
    Some(super::centered(area, desired_width, desired_height))
}

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let focus = app.help_focus();
    let Some(popup) = popup_rect(area, app.view, focus) else {
        return;
    };

    let active = app.view;
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled("?", theme::HEADER));
    let paragraph = content_paragraph(active, focus)
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
        overlay.set_frame(Rect::new(0, 0, 80, 10), View::Stories, HelpFocus::View);

        overlay.scroll_down(usize::MAX);
        let bottom = overlay.scroll_offset();
        overlay.scroll_down(1);

        assert_eq!(bottom + 8, overlay.wrapped_line_count());
        assert_eq!(overlay.scroll_offset(), bottom);
    }

    #[test]
    fn help_that_fits_in_a_tall_terminal_does_not_scroll() {
        let mut overlay = HelpOverlay::default();
        overlay.set_frame(Rect::new(0, 0, 80, 100), View::Stories, HelpFocus::View);

        overlay.scroll_down(usize::MAX);

        assert_eq!(overlay.scroll_offset(), 0);
    }

    #[test]
    fn narrow_terminal_sizes_the_popup_for_wrapped_help_lines() {
        let wide = popup_rect(Rect::new(0, 0, 80, 100), View::Stories, HelpFocus::View).unwrap();
        let narrow = popup_rect(Rect::new(0, 0, 30, 100), View::Stories, HelpFocus::View).unwrap();

        assert!(
            narrow.height > wide.height,
            "narrow help should grow from {wide:?}, got {narrow:?}"
        );
    }

    #[test]
    fn small_terminal_still_gets_a_scrollable_help_viewport() {
        let popup = popup_rect(Rect::new(0, 0, 9, 5), View::Stories, HelpFocus::View);

        assert_eq!(popup, Some(Rect::new(0, 0, 9, 5)));
    }
}
