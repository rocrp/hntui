use crate::app::App;
use crate::ui::theme;
use crate::ui::{format_age, format_error, now_unix};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let spinner = app.spinner_frame();
    let title = app
        .current_story
        .as_ref()
        .map(|s| s.title.as_str())
        .unwrap_or("Comments");
    let title = if app.comment_loading {
        format!("{title} (loading {spinner})")
    } else {
        title.to_string()
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let (list_area, footer_area) = super::list_footer_areas(inner);

    if app.comment_loading && app.comment_list.is_empty() {
        let items = vec![ListItem::new(Line::from(format!("Loading {spinner}")))];
        let list = List::new(items)
            .highlight_symbol("")
            .highlight_style(theme::SELECTED);
        let mut state = app.comment_list_state.clone();
        frame.render_stateful_widget(list, list_area, &mut state);
    } else if app.comment_list.is_empty() {
        let items = vec![ListItem::new(Line::from("No comments."))];
        let list = List::new(items)
            .highlight_symbol("")
            .highlight_style(theme::SELECTED);
        let mut state = app.comment_list_state.clone();
        frame.render_stateful_widget(list, list_area, &mut state);
    } else {
        let selected = app.comment_list_state.selected().unwrap_or(0);
        let mut visible_lines = app.comment_layout.visible_lines(selected);

        if visible_lines.is_empty() {
            visible_lines.push(Line::from(""));
        }

        frame.render_widget(Paragraph::new(visible_lines), list_area);
    }

    let footer_block = Block::default().borders(Borders::TOP);
    let footer_inner = footer_block.inner(footer_area);
    frame.render_widget(footer_block, footer_area);

    let now = now_unix();
    let show_copied = app.copied_flash.is_some_and(|t| t.elapsed().as_secs() < 2);
    let meta = if show_copied {
        Line::from(Span::styled("Copied!", theme::SUCCESS))
    } else if let Some(err) = app.last_error.as_deref() {
        Line::from(vec![Span::styled(
            format!("Error: {}", format_error(err)),
            theme::ERROR,
        )])
    } else if let Some(story) = app.current_story.as_ref() {
        let age = format_age(story.time, now);
        Line::from(format!(
            "{} pts by {} {age} | {} comments",
            story.score, story.by, story.comment_count
        ))
    } else if app.comment_loading {
        Line::from("Loading…")
    } else {
        Line::from("")
    };

    let help = Line::from(format!(
        "j/k:nav  h/←:collapse  l/→:expand  Enter/c:toggle  y:copy  s:summarize  o:comments  O:source  r:refresh  ?:help  q:back    {} comments",
        app.comment_list.len()
    ));
    frame.render_widget(Paragraph::new(vec![meta, help]), footer_inner);
}

pub(crate) fn content_areas(area: Rect) -> (Rect, Rect) {
    super::bordered_list_footer_areas(area)
}
