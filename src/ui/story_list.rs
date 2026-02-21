use crate::app::App;
use crate::ui::theme;
use crate::ui::{domain_from_url, format_age, format_error, now_unix};
use html_escape::decode_html_entities;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let spinner = app.spinner_frame();
    let title = if app.story_loading && app.stories.is_empty() {
        format!("Hacker News (loading {spinner})")
    } else if app.story_loading {
        format!("Hacker News (refreshing {spinner})")
    } else if app.prefetch_in_flight && app.has_comment_prefetch_in_flight() {
        format!("Hacker News (prefetching + comments {spinner})")
    } else if app.prefetch_in_flight {
        format!("Hacker News (prefetching {spinner})")
    } else if app.has_comment_prefetch_in_flight() {
        format!("Hacker News (preloading comments {spinner})")
    } else {
        "Hacker News".to_string()
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [list_area, footer_area] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .areas(inner);

    app.story_page_size = (list_area.height as usize).max(1);
    app.maybe_prefetch_stories();

    fn bucket_importance(value: f64) -> f64 {
        if value >= 0.85 {
            1.0
        } else if value >= 0.65 {
            0.75
        } else if value >= 0.45 {
            0.5
        } else if value >= 0.25 {
            0.25
        } else {
            0.0
        }
    }

    let selected = app.story_list_state.selected().unwrap_or(0);
    let half_viewport = (app.story_page_size / 2).max(1);

    let items = if app.story_loading && app.stories.is_empty() {
        vec![ListItem::new(Line::from(format!("Loading {spinner}")))]
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

                // Calculate importance from score + comments
                let score_level = theme::score_level(story.score);
                let comment_level = theme::comment_level(story.comment_count);
                let weighted = ((score_level * 0.7) + (comment_level * 0.3)).clamp(0.0, 1.0);
                let importance = bucket_importance(weighted);

                // Calculate distance from selected row
                let distance = idx.abs_diff(selected);

                // Get gradient color based on position, importance, and focus distance
                let fg = theme::story_gradient_fg(idx, importance, distance, half_viewport);

                // Style modifiers based on importance
                let mut base_style = Style::default().fg(fg);
                if importance >= 0.9 {
                    base_style = base_style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
                } else if importance >= 0.75 {
                    base_style = base_style.add_modifier(Modifier::BOLD);
                }

                let prefetching = app.is_comment_prefetching_for_story(story.id);

                let mut spans = vec![
                    Span::styled(format!("{:>2}. ", idx + 1), base_style),
                    Span::styled(title, base_style),
                    Span::styled(
                        format!(" ({domain})"),
                        base_style.add_modifier(Modifier::ITALIC),
                    ),
                    Span::raw("  "),
                    Span::styled(format!("{}", story.score), base_style),
                    Span::styled("·", base_style),
                    Span::styled(format!("{}", story.comment_count), base_style),
                ];

                if prefetching {
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(
                        spinner.to_string(),
                        base_style.add_modifier(Modifier::DIM),
                    ));
                }

                ListItem::new(Line::from(spans))
            })
            .collect::<Vec<_>>()
    };

    let list = List::new(items).highlight_symbol("").highlight_style(
        Style::default()
            .bg(theme::palette().surface2)
            .add_modifier(Modifier::BOLD),
    );
    frame.render_stateful_widget(list, list_area, &mut app.story_list_state);

    let footer_block = Block::default().borders(Borders::TOP);
    let footer_inner = footer_block.inner(footer_area);
    frame.render_widget(footer_block, footer_area);

    let now = now_unix();
    let meta = if let Some(err) = app.last_error.as_deref() {
        Line::from(vec![Span::styled(
            format!("Error: {}", format_error(err)),
            Style::default().fg(theme::palette().red),
        )])
    } else if let Some(story) = app.selected_story() {
        let age = format_age(story.time, now);
        let score_style = Style::default()
            .fg(theme::score_color(story.score))
            .add_modifier(Modifier::BOLD);
        let comment_style = Style::default()
            .fg(theme::comment_color(story.comment_count))
            .add_modifier(Modifier::BOLD);

        let mut spans = vec![
            Span::styled(format!("{} pts", story.score), score_style),
            Span::raw(format!(" by {} ", story.by)),
            Span::styled(
                format!("{age}"),
                Style::default().fg(theme::palette().subtext0),
            ),
            Span::raw(" | "),
            Span::styled(format!("{} comments", story.comment_count), comment_style),
        ];
        if app.prefetch_in_flight {
            spans.push(Span::raw(" | "));
            spans.push(Span::styled(
                "loading more…",
                Style::default()
                    .fg(theme::palette().subtext0)
                    .add_modifier(Modifier::ITALIC),
            ));
        }
        Line::from(spans)
    } else if app.story_loading {
        Line::from("Loading…")
    } else {
        Line::from("")
    };

    let help = Line::from(format!(
        "j/k:nav  Enter/Space/l/→:comments  o:source  O:comments  r:refresh  ?:help  q:quit    {}/{} loaded",
        app.stories.len(),
        app.story_ids.len()
    ));
    let paragraph = Paragraph::new(vec![meta, help]);
    frame.render_widget(paragraph, footer_inner);
}
