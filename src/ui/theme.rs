use ratatui::style::Color;

pub(crate) const SURFACE1: Color = Color::Rgb(69, 71, 90);
pub(crate) const OVERLAY0: Color = Color::Rgb(108, 112, 134);
pub(crate) const SUBTEXT0: Color = Color::Rgb(166, 173, 200);
pub(crate) const SUBTEXT1: Color = Color::Rgb(186, 194, 222);

pub(crate) const BLUE: Color = Color::Rgb(137, 180, 250);
pub(crate) const SAPPHIRE: Color = Color::Rgb(116, 199, 236);
pub(crate) const SKY: Color = Color::Rgb(137, 220, 235);
pub(crate) const TEAL: Color = Color::Rgb(148, 226, 213);
pub(crate) const GREEN: Color = Color::Rgb(166, 227, 161);
pub(crate) const YELLOW: Color = Color::Rgb(249, 226, 175);
pub(crate) const PEACH: Color = Color::Rgb(250, 179, 135);
pub(crate) const RED: Color = Color::Rgb(243, 139, 168);
pub(crate) const MAUVE: Color = Color::Rgb(203, 166, 247);
pub(crate) const PINK: Color = Color::Rgb(245, 194, 231);

const RAINBOW: [Color; 10] = [BLUE, SAPPHIRE, SKY, TEAL, GREEN, YELLOW, PEACH, RED, MAUVE, PINK];

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

pub(crate) fn blend(a: Color, b: Color, t: f64) -> Color {
    let (ar, ag, ab) = rgb_components(a);
    let (br, bg, bb) = rgb_components(b);
    let t = t.clamp(0.0, 1.0);

    let r = lerp_u8(ar, br, t);
    let g = lerp_u8(ag, bg, t);
    let b = lerp_u8(ab, bb, t);
    Color::Rgb(r, g, b)
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
