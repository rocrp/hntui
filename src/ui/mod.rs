pub mod comment_view;
pub mod story_list;

use crate::app::{App, View};
use ratatui::Frame;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn render(frame: &mut Frame, app: &mut App) {
    match app.view {
        View::Stories => story_list::render(frame, app),
        View::Comments => comment_view::render(frame, app),
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
