use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The commented starter config. Editing a blank file teaches nobody what the
/// keys are, so a missing config is seeded with this before the Editor opens.
const TEMPLATE: &str = include_str!("../config.toml");

#[derive(Debug, Clone)]
pub struct Config {
    stored: StoredConfig,
    path: PathBuf,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct StoredConfig {
    summarize: Option<SummarizeConfig>,
    article: Option<ArticleConfig>,
}

impl StoredConfig {
    /// Rejects values that parse as TOML but cannot work: an empty model, or a
    /// limit of zero. Serde already covers the type errors.
    fn validate(&self) -> Result<()> {
        let Some(summarize) = &self.summarize else {
            return Ok(());
        };
        if summarize.model.trim().is_empty() {
            anyhow::bail!("summarize.model must not be empty");
        }
        if summarize.max_comments == 0 {
            anyhow::bail!("summarize.max_comments must be greater than 0");
        }
        if summarize.timeout_secs == 0 {
            anyhow::bail!("summarize.timeout_secs must be greater than 0");
        }
        if summarize.max_article_chars == 0 {
            anyhow::bail!("summarize.max_article_chars must be greater than 0");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ArticleConfig {
    /// localwebrs executable; resolved through PATH when it is a bare name.
    #[serde(default = "default_article_bin")]
    pub bin: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SummarizeConfig {
    pub model: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default = "default_max_comments")]
    pub max_comments: usize,
    /// Total HTTP timeout per attempt, including the streamed response body.
    #[serde(default = "default_summary_timeout_secs")]
    pub timeout_secs: u64,
    /// Feed the story's Article to the model alongside the discussion.
    #[serde(default = "default_include_article")]
    pub include_article: bool,
    /// Head-truncation ceiling for the Article section of the prompt.
    #[serde(default = "default_max_article_chars")]
    pub max_article_chars: usize,
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueSource {
    HntuiEnv,
    File(PathBuf),
    ProviderEnv(String),
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveValue {
    pub value: Option<String>,
    pub source: ValueSource,
    pub file_shadowed: bool,
}

impl EffectiveValue {
    pub fn status(&self) -> Option<String> {
        match &self.source {
            ValueSource::HntuiEnv if self.file_shadowed => {
                Some("set by HNTUI_LLM_API_KEY — file value shadowed".to_string())
            }
            ValueSource::HntuiEnv => Some("set by HNTUI_LLM_API_KEY".to_string()),
            ValueSource::File(path) => Some(format!("set by {}", path.display())),
            ValueSource::ProviderEnv(name) => Some(format!("set by {name}")),
            ValueSource::Missing => None,
        }
    }
}

pub(crate) fn default_max_comments() -> usize {
    200
}

pub(crate) fn default_summary_timeout_secs() -> u64 {
    600
}

fn default_article_bin() -> String {
    "localwebrs".to_string()
}

pub(crate) fn default_include_article() -> bool {
    true
}

pub(crate) fn default_max_article_chars() -> usize {
    20_000
}

pub(crate) fn default_system_prompt() -> String {
    "Summarize this Hacker News discussion concisely. \
     Highlight key arguments, disagreements, and consensus points. \
     When article content is provided, ground the summary in it: say what the \
     article claims before what commenters make of it."
        .to_string()
}

impl Config {
    #[cfg(test)]
    pub(crate) fn for_test(path: PathBuf) -> Self {
        Self {
            stored: StoredConfig::default(),
            path,
        }
    }

    /// Loads one exact path, as `--config` does.
    #[cfg(test)]
    pub(crate) fn load_from_for_test(path: PathBuf) -> Result<Self> {
        Self::load_from(vec![path.clone()], path)
    }

    #[cfg(test)]
    pub(crate) fn for_test_with_summarize(path: PathBuf, summarize: SummarizeConfig) -> Self {
        Self {
            stored: StoredConfig {
                summarize: Some(summarize),
                article: None,
            },
            path,
        }
    }

    pub fn load(explicit_path: Option<&Path>) -> Result<Self> {
        if let Some(path) = explicit_path {
            return Self::load_from(vec![path.to_path_buf()], path.to_path_buf());
        }

        Self::load_from(discovery_candidates(), default_config_path()?)
    }

    fn load_from(candidates: Vec<PathBuf>, default_path: PathBuf) -> Result<Self> {
        let path = candidates
            .iter()
            .find(|candidate| candidate.exists())
            .cloned()
            .unwrap_or(default_path);
        let stored: StoredConfig = if path.exists() {
            let contents = std::fs::read_to_string(&path)
                .with_context(|| format!("read config {}", path.display()))?;
            toml::from_str(&contents).with_context(|| format!("parse config {}", path.display()))?
        } else {
            StoredConfig::default()
        };
        stored.validate()?;
        Ok(Self { stored, path })
    }

    /// The file this config was read from — the one the Editor opens.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Creates the config file from the template when it does not exist yet, so
    /// the Editor always opens something worth reading.
    pub fn ensure_file_exists(&self) -> Result<()> {
        if self.path.exists() {
            return Ok(());
        }
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create config dir {}", parent.display()))?;
        }
        std::fs::write(&self.path, TEMPLATE)
            .with_context(|| format!("write starter config {}", self.path.display()))
    }

    /// Re-reads this config's file. The caller keeps the current config when
    /// this fails, so a typo in the file never takes the app down with it.
    ///
    /// A missing file is a failure here, unlike at startup: reaching this point
    /// means the file was supposed to be there, and treating its absence as an
    /// empty config would silently wipe the settings the session is running on.
    pub fn reload(&self) -> Result<Self> {
        anyhow::ensure!(
            self.path.exists(),
            "config {} does not exist",
            self.path.display()
        );
        Self::load_from(vec![self.path.clone()], self.path.clone())
    }

    pub fn summarize(&self) -> Option<&SummarizeConfig> {
        self.stored.summarize.as_ref()
    }

    pub fn article_bin(&self) -> String {
        self.stored
            .article
            .as_ref()
            .map(|article| article.bin.clone())
            .unwrap_or_else(default_article_bin)
    }

    pub fn effective_api_key(&self) -> EffectiveValue {
        let file_value = self
            .summarize()
            .and_then(|summarize| nonempty(summarize.api_key.as_deref()));
        if let Some(value) = env_value("HNTUI_LLM_API_KEY") {
            return EffectiveValue {
                value: Some(value),
                source: ValueSource::HntuiEnv,
                file_shadowed: file_value.is_some(),
            };
        }
        if let Some(value) = file_value {
            return EffectiveValue {
                value: Some(value.to_string()),
                source: ValueSource::File(self.path.clone()),
                file_shadowed: false,
            };
        }
        if let Some(summarize) = self.summarize() {
            for model in summarize.model.split(',') {
                let provider = model
                    .trim()
                    .split_once('/')
                    .map_or_else(|| model.trim(), |(provider, _)| provider);
                if provider.is_empty() || provider == "ollama" {
                    continue;
                }
                let name = format!("{}_API_KEY", provider.to_uppercase().replace('-', "_"));
                if let Some(value) = env_value(&name) {
                    return EffectiveValue {
                        value: Some(value),
                        source: ValueSource::ProviderEnv(name),
                        file_shadowed: false,
                    };
                }
            }
        }
        EffectiveValue {
            value: None,
            source: ValueSource::Missing,
            file_shadowed: false,
        }
    }

    pub fn api_key_override(&self) -> Option<String> {
        let effective = self.effective_api_key();
        matches!(
            effective.source,
            ValueSource::HntuiEnv | ValueSource::File(_)
        )
        .then_some(effective.value)
        .flatten()
    }
}

fn discovery_candidates() -> Vec<PathBuf> {
    let mut candidates = candidates_for("config.toml");
    for candidate in candidates_for("plugin-config.toml") {
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    candidates
}

fn candidates_for(filename: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let mut push = |path: PathBuf| {
        if !candidates.contains(&path) {
            candidates.push(path);
        }
    };
    push(PathBuf::from(filename));
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            push(directory.join(filename));
        }
    }
    if let Some(project) = directories::ProjectDirs::from("dev", "hntui", "hntui") {
        push(project.config_dir().join(filename));
    }
    if let Some(home) = std::env::var_os("HOME") {
        push(
            PathBuf::from(home)
                .join(".config")
                .join("hntui")
                .join(filename),
        );
    }
    candidates
}

fn default_config_path() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("HOME") {
        return Ok(PathBuf::from(home).join(".config/hntui/config.toml"));
    }
    directories::ProjectDirs::from("dev", "hntui", "hntui")
        .map(|project| project.config_dir().join("config.toml"))
        .context("resolve config directory")
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .and_then(|value| nonempty(Some(&value)).map(str::to_string))
}

#[cfg(test)]
mod tests;
