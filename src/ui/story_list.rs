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
    let feed_label = app.current_feed.label();
    let filter_suffix = if !app.keyword_filter.is_empty() {
        let visible = app.visible_story_count();
        format!(" (filtered: {visible})")
    } else {
        String::new()
    };
    let title = if app.search_active {
        let n = app.stories.len();
        let q = &app.search_query;
        if app.story_loading {
            format!("Search: {q} (loading {spinner})")
        } else {
            format!("Search: {q} ({n} results){filter_suffix}")
        }
    } else if app.story_loading && app.stories.is_empty() {
        format!("{feed_label} (loading {spinner})")
    } else if app.story_loading {
        format!("{feed_label} (refreshing {spinner})")
    } else if app.prefetch_in_flight && app.has_comment_prefetch_in_flight() {
        format!("{feed_label} (prefetching + comments {spinner}){filter_suffix}")
    } else if app.prefetch_in_flight {
        format!("{feed_label} (prefetching {spinner}){filter_suffix}")
    } else if app.has_comment_prefetch_in_flight() {
        format!("{feed_label} (preloading comments {spinner}){filter_suffix}")
    } else {
        format!("{feed_label}{filter_suffix}")
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

    let use_filter = !app.keyword_filter.is_empty();
    let visible_count = app.visible_story_count();

    let items = if app.story_loading && app.stories.is_empty() {
        vec![ListItem::new(Line::from(format!("Loading {spinner}")))]
    } else if visible_count == 0 && use_filter {
        vec![ListItem::new(Line::from(
            "No stories match filter. Press f to change.",
        ))]
    } else if app.stories.is_empty() {
        vec![ListItem::new(Line::from(
            "No stories loaded. Press r to refresh.",
        ))]
    } else {
        // Pre-collect data to avoid borrow conflicts with story_list_state
        let story_data: Vec<_> = (0..visible_count)
            .map(|idx| {
                let story_idx = if use_filter {
                    app.visible_story_indices[idx]
                } else {
                    idx
                };
                let story = &app.stories[story_idx];
                (
                    idx,
                    story.id,
                    story.title.clone(),
                    story.url.clone(),
                    story.score,
                    story.comment_count,
                    app.is_comment_prefetching_for_story(story.id),
                )
            })
            .collect();

        story_data
            .into_iter()
            .map(|(idx, _id, title, url, score, comment_count, prefetching)| {
                let domain = url
                    .as_deref()
                    .and_then(domain_from_url)
                    .unwrap_or_else(|| "self".to_string());
                let title = decode_html_entities(&title).into_owned();

                let score_level = theme::score_level(score);
                let comment_level = theme::comment_level(comment_count);
                let weighted = ((score_level * 0.7) + (comment_level * 0.3)).clamp(0.0, 1.0);
                let importance = bucket_importance(weighted);

                let distance = idx.abs_diff(selected);
                let fg = theme::story_gradient_fg(idx, importance, distance, half_viewport);

                let mut base_style = Style::default().fg(fg);
                if importance >= 0.9 {
                    base_style = base_style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
                } else if importance >= 0.75 {
                    base_style = base_style.add_modifier(Modifier::BOLD);
                }

                let mut spans = vec![
                    Span::styled(format!("{:>2}. ", idx + 1), base_style),
                    Span::styled(title, base_style),
                    Span::styled(
                        format!(" ({domain})"),
                        base_style.add_modifier(Modifier::ITALIC),
                    ),
                    Span::raw("  "),
                    Span::styled(format!("{}", score), base_style),
                    Span::styled("·", base_style),
                    Span::styled(format!("{}", comment_count), base_style),
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
    let meta = if app.search_input_active {
        let cursor = format!("/ {}│", app.search_query);
        Line::from(vec![
            Span::styled(
                cursor,
                Style::default()
                    .fg(theme::palette().text)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                "Enter:search  Esc:cancel",
                Style::default().fg(theme::palette().subtext0),
            ),
        ])
    } else if let Some(err) = app.last_error.as_deref() {
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

    let help = if app.search_input_active || app.search_active {
        Line::from(
            "j/k:nav  Enter/Space/l/→:comments  o:source  /:search  f:feeds  Esc:back to feed  ?:help"
                .to_string(),
        )
    } else {
        let count_info = if use_filter {
            format!("{}/{} visible  {}/{} loaded", visible_count, app.stories.len(), app.stories.len(), app.story_ids.len())
        } else {
            format!("{}/{} loaded", app.stories.len(), app.story_ids.len())
        };
        Line::from(format!(
            "j/k:nav  Enter/Space/l/→:comments  o:source  O:comments  /:search  f:feeds  r:refresh  ?:help  q:quit    {count_info}"
        ))
    };
    let paragraph = Paragraph::new(vec![meta, help]);
    frame.render_widget(paragraph, footer_inner);
}
