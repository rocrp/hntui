use anyhow::{anyhow, ensure, Context, Result};
use ratatui::style::Color;
use serde::Deserialize;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

static THEME: OnceLock<Theme> = OnceLock::new();

#[derive(Debug, Clone)]
pub(crate) struct Theme {
    pub(crate) palette: Palette,
    pub(crate) layout: Layout,
    pub(crate) typography: Typography,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct Palette {
    pub(crate) surface2: Color,
    pub(crate) overlay0: Color,
    pub(crate) subtext0: Color,
    pub(crate) subtext1: Color,
    pub(crate) text: Color,
    pub(crate) blue: Color,
    pub(crate) sapphire: Color,
    pub(crate) sky: Color,
    pub(crate) teal: Color,
    pub(crate) green: Color,
    pub(crate) yellow: Color,
    pub(crate) peach: Color,
    pub(crate) red: Color,
    pub(crate) mauve: Color,
    pub(crate) pink: Color,
    pub(crate) rainbow: Vec<Color>,
}

#[derive(Debug, Clone)]
pub(crate) struct Layout {
    pub(crate) comment_max_lines: usize,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct Typography {
    pub(crate) family: String,
    pub(crate) size: f32,
    pub(crate) weight: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeConfig {
    font: FontConfig,
    layout: LayoutConfig,
    palette: PaletteConfig,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FontConfig {
    family: String,
    size: f32,
    weight: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LayoutConfig {
    comment_max_lines: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PaletteConfig {
    surface2: String,
    overlay0: String,
    subtext0: String,
    subtext1: String,
    text: String,
    blue: String,
    sapphire: String,
    sky: String,
    teal: String,
    green: String,
    yellow: String,
    peach: String,
    red: String,
    mauve: String,
    pink: String,
    rainbow: Vec<String>,
}

pub(crate) fn init_from_path(path: &Path) -> Result<()> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("read ui config {}", path.display()))?;
    let config: ThemeConfig = toml::from_str(&contents).context("parse ui config toml")?;
    let theme = Theme::from_config(config)?;
    THEME
        .set(theme)
        .map_err(|_| anyhow!("ui theme already initialized"))?;
    Ok(())
}

pub(crate) fn palette() -> &'static Palette {
    &theme().palette
}

pub(crate) fn layout() -> &'static Layout {
    &theme().layout
}

#[allow(dead_code)]
pub(crate) fn typography() -> &'static Typography {
    &theme().typography
}

fn theme() -> &'static Theme {
    THEME
        .get()
        .expect("ui theme not initialized: call theme::init_from_path()")
}

impl Theme {
    fn from_config(config: ThemeConfig) -> Result<Self> {
        ensure!(config.font.size > 0.0, "font.size must be > 0");
        ensure!(
            !config.font.family.trim().is_empty(),
            "font.family must be non-empty"
        );
        ensure!(
            !config.font.weight.trim().is_empty(),
            "font.weight must be non-empty"
        );
        ensure!(
            config.layout.comment_max_lines > 0,
            "layout.comment_max_lines must be > 0"
        );

        let palette = Palette::from_config(config.palette)?;
        let layout = Layout {
            comment_max_lines: config.layout.comment_max_lines,
        };
        let typography = Typography {
            family: config.font.family,
            size: config.font.size,
            weight: config.font.weight,
        };

        Ok(Self {
            palette,
            layout,
            typography,
        })
    }
}

impl Palette {
    fn from_config(config: PaletteConfig) -> Result<Self> {
        let rainbow = parse_color_list("palette.rainbow", &config.rainbow)?;
        ensure!(!rainbow.is_empty(), "palette.rainbow must be non-empty");
        Ok(Self {
            surface2: parse_hex_color("palette.surface2", &config.surface2)?,
            overlay0: parse_hex_color("palette.overlay0", &config.overlay0)?,
            subtext0: parse_hex_color("palette.subtext0", &config.subtext0)?,
            subtext1: parse_hex_color("palette.subtext1", &config.subtext1)?,
            text: parse_hex_color("palette.text", &config.text)?,
            blue: parse_hex_color("palette.blue", &config.blue)?,
            sapphire: parse_hex_color("palette.sapphire", &config.sapphire)?,
            sky: parse_hex_color("palette.sky", &config.sky)?,
            teal: parse_hex_color("palette.teal", &config.teal)?,
            green: parse_hex_color("palette.green", &config.green)?,
            yellow: parse_hex_color("palette.yellow", &config.yellow)?,
            peach: parse_hex_color("palette.peach", &config.peach)?,
            red: parse_hex_color("palette.red", &config.red)?,
            mauve: parse_hex_color("palette.mauve", &config.mauve)?,
            pink: parse_hex_color("palette.pink", &config.pink)?,
            rainbow,
        })
    }
}

fn parse_color_list(label: &str, values: &[String]) -> Result<Vec<Color>> {
    values
        .iter()
        .enumerate()
        .map(|(idx, value)| parse_hex_color(&format!("{label}[{idx}]"), value))
        .collect::<Result<Vec<_>>>()
}

fn parse_hex_color(label: &str, value: &str) -> Result<Color> {
    let hex = value.trim();
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    ensure!(
        hex.len() == 6,
        "{label} must be 6-digit hex (got {value})"
    );
    let r = u8::from_str_radix(&hex[0..2], 16)
        .with_context(|| format!("{label} invalid red channel {value}"))?;
    let g = u8::from_str_radix(&hex[2..4], 16)
        .with_context(|| format!("{label} invalid green channel {value}"))?;
    let b = u8::from_str_radix(&hex[4..6], 16)
        .with_context(|| format!("{label} invalid blue channel {value}"))?;
    Ok(Color::Rgb(r, g, b))
}

pub(crate) fn rainbow(level: f64) -> Color {
    let level = level.clamp(0.0, 1.0);
    let colors = &theme().palette.rainbow;
    let max_idx = colors.len() - 1;
    let pos = level * (max_idx as f64);
    let idx = pos.floor() as usize;
    if idx >= max_idx {
        return colors[max_idx];
    }
    let t = pos - (idx as f64);
    blend(colors[idx], colors[idx + 1], t)
}

pub(crate) fn rainbow_depth(depth: usize) -> Color {
    let colors = &theme().palette.rainbow;
    let idx = (depth.saturating_mul(3)) % colors.len();
    colors[idx]
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
