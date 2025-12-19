use anyhow::{anyhow, ensure, Context, Result};
use ratatui::style::Color;
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static THEME: OnceLock<Theme> = OnceLock::new();
const DEFAULT_UI_CONFIG_TOML: &str = include_str!("../../ui-config.toml");

#[derive(Debug, Clone)]
pub(crate) struct Theme {
    pub(crate) palette: Palette,
    pub(crate) layout: Layout,
    pub(crate) score_scale: Scale,
    pub(crate) comment_scale: Scale,
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
    pub(crate) comment_max_lines: Option<usize>,
    pub(crate) comment_default_visible_levels: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct Scale {
    steps: Vec<ScaleStep>,
}

#[derive(Debug, Clone)]
struct ScaleStep {
    min: i64,
    color: Color,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeConfig {
    layout: LayoutConfig,
    palette: PaletteConfig,
    score_scale: ScaleConfig,
    comment_scale: ScaleConfig,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LayoutConfig {
    comment_max_lines: i64,
    comment_default_visible_levels: usize,
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScaleConfig {
    steps: Vec<ScaleStepConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScaleStepConfig {
    min: i64,
    color: String,
}

pub(crate) fn init_from_candidates(
    paths: &[PathBuf],
    allow_default: bool,
) -> Result<Option<PathBuf>> {
    ensure!(!paths.is_empty(), "ui config search paths must be non-empty");
    for path in paths {
        if !path.exists() {
            continue;
        }
        init_from_path(path)?;
        return Ok(Some(path.clone()));
    }

    if allow_default {
        init_from_str("built-in ui config", DEFAULT_UI_CONFIG_TOML)?;
        return Ok(None);
    }

    let tried = paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(anyhow!("ui config not found; tried: {tried}"))
}

fn init_from_path(path: &Path) -> Result<()> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("read ui config {}", path.display()))?;
    init_from_str(&format!("ui config {}", path.display()), &contents)
}

fn init_from_str(label: &str, contents: &str) -> Result<()> {
    let raw: toml::Value =
        toml::from_str(contents).with_context(|| format!("parse {label} toml"))?;
    if raw.get("font").is_some() {
        return Err(anyhow!(
            "ui config no longer supports [font]; remove the [font] section and set font in your terminal emulator"
        ));
    }
    let config: ThemeConfig = raw
        .try_into()
        .with_context(|| format!("decode {label} toml"))?;
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

pub(crate) fn score_color(score: i64) -> Color {
    theme().score_scale.color_for(score)
}

pub(crate) fn comment_color(comments: i64) -> Color {
    theme().comment_scale.color_for(comments)
}

fn theme() -> &'static Theme {
    THEME
        .get()
        .expect("ui theme not initialized: call theme::init_from_path()")
}

impl Theme {
    fn from_config(config: ThemeConfig) -> Result<Self> {
        let comment_max_lines = if config.layout.comment_max_lines == -1 {
            None
        } else {
            ensure!(
                config.layout.comment_max_lines > 0,
                "layout.comment_max_lines must be > 0 or -1"
            );
            let value = usize::try_from(config.layout.comment_max_lines)
                .with_context(|| "layout.comment_max_lines overflow")?;
            Some(value)
        };
        ensure!(
            config.layout.comment_default_visible_levels > 0,
            "layout.comment_default_visible_levels must be > 0"
        );

        let palette = Palette::from_config(config.palette)?;
        let layout = Layout {
            comment_max_lines,
            comment_default_visible_levels: config.layout.comment_default_visible_levels,
        };
        let score_scale = Scale::from_config("score_scale", config.score_scale)?;
        let comment_scale = Scale::from_config("comment_scale", config.comment_scale)?;

        Ok(Self {
            palette,
            layout,
            score_scale,
            comment_scale,
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

impl Scale {
    fn from_config(label: &str, config: ScaleConfig) -> Result<Self> {
        ensure!(
            !config.steps.is_empty(),
            "{label}.steps must be non-empty"
        );
        let mut steps = Vec::with_capacity(config.steps.len());
        let mut prev_min: Option<i64> = None;
        for (idx, step) in config.steps.into_iter().enumerate() {
            ensure!(
                step.min >= 0,
                "{label}.steps[{idx}].min must be >= 0"
            );
            if let Some(prev) = prev_min {
                ensure!(
                    step.min < prev,
                    "{label}.steps[{idx}].min must be < previous min {prev}"
                );
            }
            let color =
                parse_hex_color(&format!("{label}.steps[{idx}].color"), &step.color)?;
            steps.push(ScaleStep {
                min: step.min,
                color,
            });
            prev_min = Some(step.min);
        }
        let last = steps.last().expect("scale steps non-empty");
        ensure!(last.min == 0, "{label}.steps last min must be 0");
        Ok(Self { steps })
    }

    fn color_for(&self, value: i64) -> Color {
        for step in &self.steps {
            if value >= step.min {
                return step.color;
            }
        }
        self.steps
            .last()
            .expect("scale steps must be non-empty")
            .color
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
