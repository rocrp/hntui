use crate::app::App;
use crate::ui::{domain_from_url, format_age, now_unix};
use html_escape::decode_html_entities;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let title = if app.story_loading && app.stories.is_empty() {
        "Hacker News (loading)"
    } else if app.prefetch_in_flight {
        "Hacker News (prefetching)"
    } else {
        "Hacker News"
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [list_area, footer_area] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .areas(inner);

    app.story_page_size = (list_area.height as usize).max(1);

    let items = if app.story_loading && app.stories.is_empty() {
        vec![ListItem::new(Line::from("Loading…"))]
    } else if app.stories.is_empty() {
        vec![ListItem::new(Line::from(
            "No stories loaded. Press r to refresh.",
        ))]
    } else {
        app.stories
            .iter()
            .enumerate()
            .map(|(idx, story)| {
                let domain = story
                    .url
                    .as_deref()
                    .and_then(domain_from_url)
                    .unwrap_or_else(|| "self".to_string());
                let title = decode_html_entities(&story.title);
                ListItem::new(Line::from(vec![
                    Span::raw(format!("{:>2}. ", idx + 1)),
                    Span::raw(title),
                    Span::styled(format!(" ({domain})"), Style::default().fg(Color::DarkGray)),
                ]))
            })
            .collect::<Vec<_>>()
    };

    let list = List::new(items)
        .highlight_symbol("▶ ")
        .highlight_style(Style::default().fg(Color::Yellow));
    frame.render_stateful_widget(list, list_area, &mut app.story_list_state);

    let footer_block = Block::default().borders(Borders::TOP);
    let footer_inner = footer_block.inner(footer_area);
    frame.render_widget(footer_block, footer_area);

    let now = now_unix();
    let meta = if let Some(err) = app.last_error.as_deref() {
        Line::from(vec![Span::styled(
            format!("Error: {err}"),
            Style::default().fg(Color::Red),
        )])
    } else if let Some(story) = app.selected_story() {
        let age = format_age(story.time, now);
        let mut s = format!(
            "{} pts by {} {age} | {} comments",
            story.score, story.by, story.comment_count
        );
        if app.prefetch_in_flight {
            s.push_str(" | loading more…");
        }
        Line::from(s)
    } else if app.story_loading {
        Line::from("Loading…")
    } else {
        Line::from("")
    };

    let help = Line::from(format!(
        "j/k:nav  Enter:comments  o:open  r:refresh  q:quit    {}/{} loaded",
        app.stories.len(),
        app.story_ids.len()
    ));
    let paragraph = Paragraph::new(vec![meta, help]);
    frame.render_widget(paragraph, footer_inner);
}
