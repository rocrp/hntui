use ratatui::style::{Color, Modifier, Style};

const COMMENT_INDENT_BLEND: f64 = 0.35;

const fn hex(rgb: u32) -> Color {
    Color::Rgb(
        ((rgb >> 16) & 0xff) as u8,
        ((rgb >> 8) & 0xff) as u8,
        (rgb & 0xff) as u8,
    )
}

// Catppuccin Frappé palette
pub(crate) const SURFACE2: Color = hex(0x414559);
pub(crate) const OVERLAY0: Color = hex(0x737994);
pub(crate) const SUBTEXT0: Color = hex(0xA5ADCE);
pub(crate) const SUBTEXT1: Color = hex(0xB5BFE2);
pub(crate) const TEXT: Color = hex(0xC6D0F5);
pub(crate) const BLUE: Color = hex(0x8CAAEE);
pub(crate) const TEAL: Color = hex(0x81C8BE);
pub(crate) const GREEN: Color = hex(0xA6D189);
pub(crate) const RED: Color = hex(0xE78284);
pub(crate) const MAUVE: Color = hex(0xCA9EE6);

const RAINBOW: [Color; 10] = [
    hex(0x8CAAEE), // blue
    hex(0x85C1DC), // sapphire
    hex(0x99D1DB), // sky
    hex(0x81C8BE), // teal
    hex(0xA6D189), // green
    hex(0xE5C890), // yellow
    hex(0xEF9F76), // peach
    hex(0xE78284), // red
    hex(0xCA9EE6), // mauve
    hex(0xF4B8E4), // pink
];

// Layout constants
pub(crate) const COMMENT_MAX_LINES: Option<usize> = None;
pub(crate) const COMMENT_DEFAULT_VISIBLE_LEVELS: usize = 2;

// Score/comment heat-map scales (descending threshold, last must be 0)
struct ScaleStep {
    min: i64,
    color: Color,
}

const SCORE_SCALE: [ScaleStep; 5] = [
    ScaleStep {
        min: 500,
        color: hex(0x9F633F),
    },
    ScaleStep {
        min: 250,
        color: hex(0x925B3B),
    },
    ScaleStep {
        min: 100,
        color: hex(0x855337),
    },
    ScaleStep {
        min: 50,
        color: hex(0x5F433A),
    },
    ScaleStep {
        min: 0,
        color: hex(0x3F2E2B),
    },
];

const COMMENT_SCALE: [ScaleStep; 5] = [
    ScaleStep {
        min: 300,
        color: hex(0x5F76A4),
    },
    ScaleStep {
        min: 200,
        color: hex(0x586E9A),
    },
    ScaleStep {
        min: 100,
        color: hex(0x516690),
    },
    ScaleStep {
        min: 50,
        color: hex(0x45526E),
    },
    ScaleStep {
        min: 0,
        color: hex(0x353E5B),
    },
];

// ── Semantic styles (shared across all views) ────────────────────────

/// Popup background
pub(crate) const POPUP: Style = Style::new().bg(SURFACE2);
/// Bold primary text (popup titles, keys)
pub(crate) const HEADER: Style = Style::new().fg(TEXT).add_modifier(Modifier::BOLD);
/// Accent header (summary overlay title)
pub(crate) const HEADER_ACCENT: Style = Style::new().fg(MAUVE).add_modifier(Modifier::BOLD);
/// Hint / secondary help text
pub(crate) const HINT: Style = Style::new().fg(SUBTEXT0);
/// Keyboard shortcut labels
pub(crate) const KEY: Style = Style::new().fg(TEXT).add_modifier(Modifier::BOLD);
/// Low-emphasis label
pub(crate) const LABEL: Style = Style::new().fg(SUBTEXT1);
/// Normal-emphasis value
pub(crate) const VALUE: Style = Style::new().fg(TEXT);
/// Purple accent (active section, cursor in popup)
pub(crate) const ACCENT: Style = Style::new().fg(MAUVE).add_modifier(Modifier::BOLD);
/// Green success flash ("Copied!", "Saved!")
pub(crate) const SUCCESS: Style = Style::new().fg(GREEN).add_modifier(Modifier::BOLD);
/// Error text
pub(crate) const ERROR: Style = Style::new().fg(RED);
/// Subtle metadata (overlay0)
pub(crate) const META: Style = Style::new().fg(OVERLAY0);
/// Selected-item highlight bg
pub(crate) const SELECTED: Style = Style::new().bg(SURFACE2);
/// Block quote text
pub(crate) const QUOTE: Style = Style::new().fg(SUBTEXT0).add_modifier(Modifier::ITALIC);
/// Block quote bar
pub(crate) const QUOTE_BAR: Style = Style::new().fg(OVERLAY0);
/// Inline / fenced code
pub(crate) const CODE: Style = Style::new().fg(TEAL);
/// List bullet / number
pub(crate) const LIST_MARKER: Style = Style::new().fg(BLUE);
/// Block cursor in editing mode
pub(crate) const BLOCK_CURSOR: Style = Style::new().fg(SURFACE2).bg(GREEN);

pub(crate) fn section_heading(active: bool) -> Style {
    if active {
        ACCENT
    } else {
        Style::new().fg(SUBTEXT0).add_modifier(Modifier::BOLD)
    }
}

// ── Scale helpers ────────────────────────────────────────────────────

pub(crate) fn score_color(score: i64) -> Color {
    scale_color(score, &SCORE_SCALE)
}

pub(crate) fn comment_color(comments: i64) -> Color {
    scale_color(comments, &COMMENT_SCALE)
}

pub(crate) fn score_level(score: i64) -> f64 {
    scale_level(score, &SCORE_SCALE)
}

pub(crate) fn comment_level(comments: i64) -> f64 {
    scale_level(comments, &COMMENT_SCALE)
}

fn scale_color(value: i64, steps: &[ScaleStep]) -> Color {
    for step in steps {
        if value >= step.min {
            return step.color;
        }
    }
    steps.last().expect("scale non-empty").color
}

fn scale_level(value: i64, steps: &[ScaleStep]) -> f64 {
    let idx = steps
        .iter()
        .position(|step| value >= step.min)
        .unwrap_or(steps.len().saturating_sub(1));
    if steps.len() == 1 {
        return 1.0;
    }
    let denom = (steps.len() - 1) as f64;
    (1.0 - (idx as f64 / denom)).clamp(0.0, 1.0)
}

// ── Rainbow / gradient helpers ───────────────────────────────────────

pub(crate) fn comment_indent_color(depth: usize) -> Color {
    let accent = rainbow_depth(depth);
    blend(OVERLAY0, accent, COMMENT_INDENT_BLEND)
}

pub(crate) fn rainbow(level: f64) -> Color {
    let level = level.clamp(0.0, 1.0);
    let max_idx = RAINBOW.len() - 1;
    let pos = level * (max_idx as f64);
    let idx = pos.floor() as usize;
    if idx >= max_idx {
        return RAINBOW[max_idx];
    }
    let t = pos - (idx as f64);
    blend(RAINBOW[idx], RAINBOW[idx + 1], t)
}

pub(crate) fn rainbow_depth(depth: usize) -> Color {
    let idx = (depth.saturating_mul(3)) % RAINBOW.len();
    RAINBOW[idx]
}

/// Story foreground based on row position, importance, and distance from selection.
pub(crate) fn story_gradient_fg(
    row_index: usize,
    importance: f64,
    distance: usize,
    half_viewport: usize,
) -> Color {
    let hue_pos = (row_index as f64 * 0.1) % 1.0;
    let rainbow_color = rainbow(hue_pos);

    let saturation = 0.2 + (importance * 0.75);
    let importance_adjusted = blend(SUBTEXT0, rainbow_color, saturation);

    if distance == 0 {
        return importance_adjusted;
    }

    let max_dist = half_viewport.max(1) as f64;
    let fade = (distance as f64 / max_dist).min(1.0);
    let dim_factor = fade * (0.5 - importance * 0.2);
    blend(importance_adjusted, SUBTEXT0, dim_factor)
}

/// Darken a color by blending toward black.
pub(crate) fn dim_color(color: Color, factor: f64) -> Color {
    match color {
        Color::Rgb(r, g, b) => blend(Color::Rgb(r, g, b), Color::Rgb(0, 0, 0), factor),
        _ => Color::Rgb(30, 30, 30),
    }
}

pub(crate) fn blend(a: Color, b: Color, t: f64) -> Color {
    let (ar, ag, ab) = rgb_components(a);
    let (br, bg, bb) = rgb_components(b);
    let t = t.clamp(0.0, 1.0);
    Color::Rgb(lerp_u8(ar, br, t), lerp_u8(ag, bg, t), lerp_u8(ab, bb, t))
}

fn rgb_components(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => panic!("expected Color::Rgb"),
    }
}

fn lerp_u8(a: u8, b: u8, t: f64) -> u8 {
    let a = a as f64;
    let b = b as f64;
    ((a + ((b - a) * t)).round()).clamp(0.0, 255.0) as u8
}
