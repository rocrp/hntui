use crate::app::App;
use crate::ui::{domain_from_url, format_age, now_unix};
use crate::ui::theme;
use html_escape::decode_html_entities;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let spinner = app.spinner_frame();
    let title = if app.story_loading && app.stories.is_empty() {
        format!("Hacker News (loading {spinner})")
    } else if app.prefetch_in_flight && app.comment_prefetch_in_flight {
        format!("Hacker News (prefetching + comments {spinner})")
    } else if app.prefetch_in_flight {
        format!("Hacker News (prefetching {spinner})")
    } else if app.comment_prefetch_in_flight {
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

    fn normalize_i64(value: i64, min: i64, max: i64) -> f64 {
        if max <= min {
            return 0.0;
        }
        let numer = (value.saturating_sub(min)) as f64;
        let denom = (max - min) as f64;
        (numer / denom).clamp(0.0, 1.0)
    }

    #[derive(Debug, Clone, Copy)]
    struct StoryStats {
        min_score: i64,
        max_score: i64,
        min_comments: i64,
        max_comments: i64,
    }

    let stats = if app.stories.is_empty() {
        None
    } else {
        let mut min_score = i64::MAX;
        let mut max_score = i64::MIN;
        let mut min_comments = i64::MAX;
        let mut max_comments = i64::MIN;
        for story in &app.stories {
            min_score = min_score.min(story.score);
            max_score = max_score.max(story.score);
            min_comments = min_comments.min(story.comment_count);
            max_comments = max_comments.max(story.comment_count);
        }
        Some(StoryStats {
            min_score,
            max_score,
            min_comments,
            max_comments,
        })
    };

    let items = if app.story_loading && app.stories.is_empty() {
        vec![ListItem::new(Line::from(format!("Loading {spinner}")))]
    } else if app.stories.is_empty() {
        vec![ListItem::new(Line::from(
            "No stories loaded. Press r to refresh.",
        ))]
    } else {
        let stats = stats.expect("stats present when stories present");

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

                let score_level = normalize_i64(story.score, stats.min_score, stats.max_score);
                let comment_level = normalize_i64(
                    story.comment_count,
                    stats.min_comments,
                    stats.max_comments,
                );
                let importance = ((score_level * 0.7) + (comment_level * 0.3)).clamp(0.0, 1.0);

                let accent = theme::rainbow(importance);

                let title_color = theme::blend(theme::OVERLAY0, theme::TEXT, importance);
                let mut title_style = Style::default().fg(title_color);
                if importance >= 0.9 {
                    title_style = title_style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
                } else if importance >= 0.75 {
                    title_style = title_style.add_modifier(Modifier::BOLD);
                }

                let mut score_style = Style::default().fg(theme::rainbow(score_level));
                if score_level >= 0.85 {
                    score_style = score_style.add_modifier(Modifier::BOLD);
                }

                let mut comment_style = Style::default().fg(theme::rainbow(comment_level));
                if comment_level >= 0.85 {
                    comment_style = comment_style.add_modifier(Modifier::BOLD);
                }

                ListItem::new(Line::from(vec![
                    Span::styled("▌ ", Style::default().fg(accent)),
                    Span::styled(
                        format!("{:>2}. ", idx + 1),
                        Style::default().fg(theme::SUBTEXT1),
                    ),
                    Span::styled(title, title_style),
                    Span::styled(
                        format!(" ({domain})"),
                        Style::default()
                            .fg(theme::SUBTEXT0)
                            .add_modifier(Modifier::ITALIC),
                    ),
                    Span::raw("  "),
                    Span::styled(format!("{}", story.score), score_style),
                    Span::styled("·", Style::default().fg(theme::OVERLAY0)),
                    Span::styled(format!("{}", story.comment_count), comment_style),
                ]))
            })
            .collect::<Vec<_>>()
    };

    let list = List::new(items)
        .highlight_symbol("▶ ")
        .highlight_style(
            Style::default()
                .bg(theme::SURFACE0)
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        );
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
        let stats = stats.expect("stats present when selected story present");
        let score_level = normalize_i64(story.score, stats.min_score, stats.max_score);
        let comment_level = normalize_i64(
            story.comment_count,
            stats.min_comments,
            stats.max_comments,
        );

        let score_style = Style::default()
            .fg(theme::rainbow(score_level))
            .add_modifier(Modifier::BOLD);
        let comment_style = Style::default()
            .fg(theme::rainbow(comment_level))
            .add_modifier(Modifier::BOLD);

        let mut spans = vec![
            Span::styled(format!("{} pts", story.score), score_style),
            Span::raw(format!(" by {} ", story.by)),
            Span::styled(format!("{age}"), Style::default().fg(theme::SUBTEXT0)),
            Span::raw(" | "),
            Span::styled(format!("{} comments", story.comment_count), comment_style),
        ];
        if app.prefetch_in_flight {
            spans.push(Span::raw(" | "));
            spans.push(Span::styled(
                "loading more…",
                Style::default()
                    .fg(theme::SUBTEXT0)
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
        "j/k:nav  Enter:comments  o:open  r:refresh  q:quit    {}/{} loaded",
        app.stories.len(),
        app.story_ids.len()
    ));
    let paragraph = Paragraph::new(vec![meta, help]);
    frame.render_widget(paragraph, footer_inner);
}
