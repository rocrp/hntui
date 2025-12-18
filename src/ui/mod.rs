pub mod comment_view;
pub mod story_list;

use crate::app::{App, View};
use ratatui::style::Color;
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

pub(crate) fn rainbow_color(level: f64) -> Color {
    let level = level.clamp(0.0, 1.0);
    let hue = (1.0 - level) * 270.0;
    let saturation = 0.85 + (level * 0.15);
    let lightness = 0.56 + (level * 0.14);
    hsl_to_rgb(hue, saturation, lightness)
}

pub(crate) fn rainbow_depth_color(depth: usize) -> Color {
    let hue = 270.0 - ((depth as f64) * 28.0);
    hsl_to_rgb(hue, 0.95, 0.68)
}

fn hsl_to_rgb(hue_degrees: f64, saturation: f64, lightness: f64) -> Color {
    let hue_degrees = hue_degrees.rem_euclid(360.0);
    let saturation = saturation.clamp(0.0, 1.0);
    let lightness = lightness.clamp(0.0, 1.0);

    if saturation == 0.0 {
        let v = (lightness * 255.0).round() as u8;
        return Color::Rgb(v, v, v);
    }

    let c = (1.0 - ((2.0 * lightness) - 1.0).abs()) * saturation;
    let h_prime = hue_degrees / 60.0;
    let x = c * (1.0 - ((h_prime % 2.0) - 1.0).abs());

    let (r1, g1, b1) = if h_prime < 1.0 {
        (c, x, 0.0)
    } else if h_prime < 2.0 {
        (x, c, 0.0)
    } else if h_prime < 3.0 {
        (0.0, c, x)
    } else if h_prime < 4.0 {
        (0.0, x, c)
    } else if h_prime < 5.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    let m = lightness - (c / 2.0);
    let r = ((r1 + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    let g = ((g1 + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    let b = ((b1 + m) * 255.0).round().clamp(0.0, 255.0) as u8;

    Color::Rgb(r, g, b)
}
