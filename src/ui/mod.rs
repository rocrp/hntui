pub mod comment_layout;
pub mod comment_view;
pub mod feed_filter;
pub mod help;
pub mod markdown;
pub mod settings;
pub mod story_list;
pub mod summary_overlay;
pub mod theme;

use crate::app::{App, View};
use crate::input::InputLayer;
use crate::logging;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders, Widget};
use ratatui::Frame;
use std::time::{SystemTime, UNIX_EPOCH};

const DIM_FACTOR: f64 = 0.6;

/// Widget that dims all cells in the given area by blending toward black.
struct Dim;

impl Widget for Dim {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                let cell = &mut buf[(x, y)];
                cell.fg = theme::dim_color(cell.fg, DIM_FACTOR);
                cell.bg = theme::dim_color(cell.bg, DIM_FACTOR);
            }
        }
    }
}

pub fn render(frame: &mut Frame, app: &App) {
    match app.view {
        View::Stories => story_list::render(frame, app),
        View::Comments => comment_view::render(frame, app),
    }

    let layer = app.input_layer();
    let has_overlay = matches!(
        layer,
        InputLayer::Help
            | InputLayer::Summary
            | InputLayer::FeedFilter
            | InputLayer::Settings
            | InputLayer::SettingsEditor
    );
    if has_overlay {
        frame.render_widget(Dim, frame.area());
    }

    match layer {
        InputLayer::Help => help::render(frame, app),
        InputLayer::Summary => {
            summary_overlay::render(frame, &app.summary_overlay, app.spinner_frame());
        }
        InputLayer::FeedFilter => feed_filter::render(frame, app),
        InputLayer::Settings | InputLayer::SettingsEditor => settings::render(frame, app),
        InputLayer::FilterText | InputLayer::SearchText | InputLayer::View => {}
    }
}

pub(crate) fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .try_into()
        .unwrap_or(i64::MAX)
}

pub(crate) fn format_age(item_time: i64, now: i64) -> String {
    let diff = now.saturating_sub(item_time).max(0);
    if diff < 60 {
        return format!("{diff}s");
    }
    let minutes = diff / 60;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let hours = minutes / 60;
    if hours < 48 {
        return format!("{hours}h");
    }
    let days = hours / 24;
    if days < 365 {
        return format!("{days}d");
    }
    let years = days / 365;
    format!("{years}y")
}

pub(crate) fn domain_from_url(url: &str) -> Option<String> {
    let without_scheme = url.split("://").nth(1).unwrap_or(url);
    let host_port = without_scheme.split('/').next()?;
    let host_port = host_port.split('@').next_back().unwrap_or(host_port);
    Some(host_port.trim_start_matches("www.").to_string())
}

/// Glyph used when the domain is not in the whitelist.
pub(crate) const FALLBACK_DOMAIN_ICON: &str = "\u{f0ac}";

/// Map a normalized domain (as returned by `domain_from_url`, or the literal
/// `"self"` for self-posts) to a Nerd Font glyph. Returns `None` if the
/// domain is not in the whitelist; callers can then render
/// `FALLBACK_DOMAIN_ICON` and keep the literal domain text alongside it.
pub(crate) fn domain_icon(domain: &str) -> Option<&'static str> {
    const NEWS: &str = "\u{f1ea}";
    const ACADEMIC: &str = "\u{f02d}";
    const AI: &str = "\u{f544}";
    const GITHUB: &str = "\u{f09b}";
    const GOOGLE: &str = "\u{f1a0}";
    const APPLE: &str = "\u{f179}";
    const WINDOWS: &str = "\u{f17a}";
    const AMAZON: &str = "\u{f270}";
    const STACKOVERFLOW: &str = "\u{f16c}";
    const TWITTER: &str = "\u{f099}";

    let glyph = match domain {
        "self" => "\u{f075}",

        // Code hosting
        "github.com" | "github.blog" | "gist.github.com" => GITHUB,
        "gitlab.com" => "\u{f296}",
        "bitbucket.org" => "\u{f171}",
        "codeberg.org" => "\u{f1d3}",

        // Video
        "youtube.com" | "youtu.be" => "\u{f167}",
        "twitch.tv" => "\u{f1e8}",

        // Social
        "twitter.com" | "x.com" | "bsky.app" => TWITTER,

        // Y Combinator / HN
        "news.ycombinator.com" | "ycombinator.com" => "\u{f1d4}",

        // Blog platforms
        "medium.com" => "\u{f23a}",

        // Q&A
        "stackoverflow.com" => STACKOVERFLOW,

        // Big tech
        "googleblog.com" | "ai.google" | "research.google" | "blog.google" => GOOGLE,
        "aws.amazon.com" | "amazon.com" | "amazon.science" => AMAZON,

        // AI labs
        "anthropic.com" | "claude.ai" | "openai.com" | "chatgpt.com" | "huggingface.co"
        | "mistral.ai" | "cohere.com" | "cohere.ai" | "deepmind.com" | "deepmind.google" => AI,

        // News
        "nytimes.com"
        | "bloomberg.com"
        | "wsj.com"
        | "ft.com"
        | "theguardian.com"
        | "reuters.com"
        | "washingtonpost.com"
        | "economist.com"
        | "theatlantic.com"
        | "apnews.com"
        | "ap.org"
        | "npr.org"
        | "axios.com"
        | "politico.com"
        | "cnn.com"
        | "vox.com"
        | "propublica.org"
        | "theintercept.com"
        | "restofworld.org"
        | "qz.com"
        | "heise.de"
        | "scmp.com"
        | "nymag.com"
        | "newsweek.com"
        | "wired.com"
        | "vice.com"
        | "technologyreview.com"
        | "theregister.com"
        | "theverge.com"
        | "arstechnica.com"
        | "cnbc.com"
        | "engadget.com"
        | "techcrunch.com"
        | "gizmodo.com"
        | "bbc.com"
        | "bbc.co.uk"
        | "quantamagazine.org"
        | "atlasobscura.com"
        | "phys.org"
        | "apod.nasa.gov"
        | "nasa.gov" => NEWS,

        // Academic / research
        "arxiv.org"
        | "dl.acm.org"
        | "ieee.org"
        | "nature.com"
        | "science.org"
        | "sciencemag.org"
        | "biorxiv.org"
        | "medrxiv.org"
        | "pubmed.ncbi.nlm.nih.gov"
        | "scholar.google.com"
        | "zenodo.org"
        | "osf.io"
        | "ssrn.com"
        | "plos.org"
        | "pnas.org"
        | "elifesciences.org"
        | "link.springer.com"
        | "sciencedirect.com"
        | "cell.com" => ACADEMIC,

        // Suffix matches (must come after exact matches that would otherwise
        // be subsumed, e.g. scholar.google.com).
        d if d.ends_with(".github.io") => GITHUB,
        d if d == "reddit.com" || d.ends_with(".reddit.com") => "\u{f281}",
        d if d == "wikipedia.org" || d.ends_with(".wikipedia.org") => "\u{f266}",
        d if d == "stackexchange.com" || d.ends_with(".stackexchange.com") => STACKOVERFLOW,
        d if d == "substack.com" || d.ends_with(".substack.com") => "\u{f09e}",
        d if d == "apple.com" || d.ends_with(".apple.com") => APPLE,
        d if d == "microsoft.com" || d.ends_with(".microsoft.com") => WINDOWS,

        _ => return None,
    };
    Some(glyph)
}

pub(crate) fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x.saturating_add(area.width.saturating_sub(width) / 2);
    let y = area
        .y
        .saturating_add(area.height.saturating_sub(height) / 2);
    Rect {
        x,
        y,
        width,
        height,
    }
}

pub(crate) fn bordered_list_footer_areas(area: Rect) -> (Rect, Rect) {
    let inner = Block::default().borders(Borders::ALL).inner(area);
    list_footer_areas(inner)
}

pub(crate) fn list_footer_areas(inner: Rect) -> (Rect, Rect) {
    let [list_area, footer_area] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .areas(inner);
    (list_area, footer_area)
}

pub(crate) fn format_error(err: &str) -> String {
    let mut out = String::from(err);
    if let Some(tip) = error_tip(err) {
        out.push_str(" | tip: ");
        out.push_str(tip);
    }
    if let Some(path) = logging::log_path() {
        out.push_str(" | log: ");
        out.push_str(&path.to_string_lossy());
    }
    out
}

fn error_tip(err: &str) -> Option<&'static str> {
    let lower = err.to_ascii_lowercase();
    if lower.contains("too many open files") || lower.contains("os error 24") {
        return Some("--concurrency 8 or --no-file-cache");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_icon_known_host() {
        assert_eq!(domain_icon("github.com"), Some("\u{f09b}"));
    }

    #[test]
    fn domain_icon_suffix_match_reddit() {
        assert_eq!(domain_icon("old.reddit.com"), Some("\u{f281}"));
        assert_eq!(domain_icon("reddit.com"), Some("\u{f281}"));
    }

    #[test]
    fn domain_icon_suffix_match_wikipedia() {
        assert_eq!(domain_icon("en.wikipedia.org"), Some("\u{f266}"));
    }

    #[test]
    fn domain_icon_self_post() {
        assert_eq!(domain_icon("self"), Some("\u{f075}"));
    }

    #[test]
    fn domain_icon_unknown_returns_none() {
        assert_eq!(domain_icon("some-random-blog.example"), None);
    }

    #[test]
    fn domain_icon_github_family() {
        assert_eq!(domain_icon("github.blog"), Some("\u{f09b}"));
        assert_eq!(domain_icon("gist.github.com"), Some("\u{f09b}"));
        assert_eq!(domain_icon("user.github.io"), Some("\u{f09b}"));
    }

    #[test]
    fn domain_icon_news_bucket() {
        assert_eq!(domain_icon("theguardian.com"), Some("\u{f1ea}"));
        assert_eq!(domain_icon("ft.com"), Some("\u{f1ea}"));
        assert_eq!(domain_icon("technologyreview.com"), Some("\u{f1ea}"));
    }

    #[test]
    fn domain_icon_academic_bucket() {
        assert_eq!(domain_icon("dl.acm.org"), Some("\u{f02d}"));
        assert_eq!(domain_icon("nature.com"), Some("\u{f02d}"));
        assert_eq!(domain_icon("scholar.google.com"), Some("\u{f02d}"));
    }

    #[test]
    fn domain_icon_ai_labs() {
        assert_eq!(domain_icon("anthropic.com"), Some("\u{f544}"));
        assert_eq!(domain_icon("huggingface.co"), Some("\u{f544}"));
    }

    #[test]
    fn domain_icon_big_tech_suffix() {
        assert_eq!(domain_icon("apple.com"), Some("\u{f179}"));
        assert_eq!(domain_icon("developer.apple.com"), Some("\u{f179}"));
        assert_eq!(domain_icon("microsoft.com"), Some("\u{f17a}"));
        assert_eq!(domain_icon("devblogs.microsoft.com"), Some("\u{f17a}"));
    }
}
