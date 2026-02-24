use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct PluginConfig {
    pub summarize: Option<SummarizeConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SummarizeConfig {
    pub api_url: String,
    pub model: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_max_comments")]
    pub max_comments: usize,
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
}

fn default_max_comments() -> usize {
    200
}

fn default_system_prompt() -> String {
    "Summarize this Hacker News discussion concisely. \
     Highlight key arguments, disagreements, and consensus points."
        .to_string()
}

impl SummarizeConfig {
    /// Resolve API key: env var `HNTUI_LLM_API_KEY` takes precedence over config field.
    pub fn resolve_api_key(&self) -> Option<String> {
        if let Ok(key) = std::env::var("HNTUI_LLM_API_KEY") {
            if !key.trim().is_empty() {
                return Some(key);
            }
        }
        if !self.api_key.trim().is_empty() {
            return Some(self.api_key.clone());
        }
        None
    }
}

pub fn load_plugin_config(candidates: &[PathBuf]) -> Result<Option<PluginConfig>> {
    for path in candidates {
        if !path.exists() {
            continue;
        }
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("read plugin config {}", path.display()))?;
        let config: PluginConfig = toml::from_str(&contents)
            .with_context(|| format!("parse plugin config {}", path.display()))?;
        return Ok(Some(config));
    }
    Ok(None)
}
