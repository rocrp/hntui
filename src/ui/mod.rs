pub mod comment_view;
pub mod help;
pub mod markdown;
pub mod plugin_overlay;
pub mod story_list;
pub mod theme;

use crate::app::{App, View};
use crate::logging;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;
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

pub fn render(frame: &mut Frame, app: &mut App) {
    match app.view {
        View::Stories => story_list::render(frame, app),
        View::Comments => comment_view::render(frame, app),
    }

    let has_overlay = app.summarize_plugin.is_overlay_visible() || app.help_visible;
    if has_overlay {
        frame.render_widget(Dim, frame.area());
    }

    let spinner = app.spinner_frame();
    plugin_overlay::render(frame, &mut app.summarize_plugin, spinner);
    if app.help_visible {
        help::render(frame, app);
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
