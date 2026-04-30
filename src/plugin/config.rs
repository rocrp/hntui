use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Serialize)]
pub struct PluginConfig {
    pub summarize: Option<SummarizeConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SummarizeConfig {
    /// smolllm `provider/model` string (or comma-separated for fallback).
    pub model: String,

    /// Optional API key override. If `None`, smolllm resolves from
    /// `{PROVIDER}_API_KEY` env var.
    #[serde(default)]
    pub api_key: Option<String>,

    /// Optional base URL override. If `None`, smolllm uses the provider's
    /// default (or `{PROVIDER}_BASE_URL` env var).
    #[serde(default)]
    pub base_url: Option<String>,

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
    /// Resolve API key: env var `HNTUI_LLM_API_KEY` > config field > `None`
    /// (in which case smolllm will try `{PROVIDER}_API_KEY` itself).
    pub fn resolve_api_key(&self) -> Option<String> {
        if let Ok(key) = std::env::var("HNTUI_LLM_API_KEY") {
            if !key.trim().is_empty() {
                return Some(key);
            }
        }
        self.api_key.as_ref().and_then(|k| {
            let trimmed = k.trim();
            (!trimmed.is_empty()).then(|| k.clone())
        })
    }
}

pub fn default_config_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config/hntui/config.toml"))
}

pub async fn save_plugin_config(path: &Path, config: &PluginConfig) -> Result<()> {
    let contents = toml::to_string_pretty(config).context("serialize plugin config")?;
    let parent = path.parent().context("config path has no parent dir")?;
    tokio::fs::create_dir_all(parent)
        .await
        .with_context(|| format!("create dir {}", parent.display()))?;

    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("system time")?
        .as_nanos();
    let tmp = path.with_extension(format!("toml.tmp.{}.{unique}", std::process::id()));
    tokio::fs::write(&tmp, contents.as_bytes())
        .await
        .with_context(|| format!("write temp {}", tmp.display()))?;
    tokio::fs::rename(&tmp, path)
        .await
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config() {
        let src = r#"
            [summarize]
            model = "gemini/gemini-flash-lite-latest"
        "#;
        let cfg: PluginConfig = toml::from_str(src).expect("parse");
        let s = cfg.summarize.expect("summarize present");
        assert_eq!(s.model, "gemini/gemini-flash-lite-latest");
        assert!(s.api_key.is_none());
        assert!(s.base_url.is_none());
        assert_eq!(s.max_comments, 200);
        assert!(s.system_prompt.contains("Summarize"));
    }

    #[test]
    fn parses_full_config() {
        let src = r#"
            [summarize]
            model = "openai/gpt-4o-mini"
            api_key = "sk-test"
            base_url = "https://example.com"
            max_comments = 50
            system_prompt = "be terse"
        "#;
        let cfg: PluginConfig = toml::from_str(src).expect("parse");
        let s = cfg.summarize.expect("summarize present");
        assert_eq!(s.api_key.as_deref(), Some("sk-test"));
        assert_eq!(s.base_url.as_deref(), Some("https://example.com"));
        assert_eq!(s.max_comments, 50);
        assert_eq!(s.system_prompt, "be terse");
    }

    #[test]
    fn resolve_api_key_prefers_env_var() {
        let var = "HNTUI_LLM_API_KEY";
        let prev = std::env::var(var).ok();
        std::env::set_var(var, "from-env");
        let cfg = SummarizeConfig {
            model: "openai/x".into(),
            api_key: Some("from-config".into()),
            base_url: None,
            max_comments: 200,
            system_prompt: String::new(),
        };
        assert_eq!(cfg.resolve_api_key().as_deref(), Some("from-env"));
        match prev {
            Some(v) => std::env::set_var(var, v),
            None => std::env::remove_var(var),
        }
    }

    #[test]
    fn resolve_api_key_falls_back_to_config() {
        let var = "HNTUI_LLM_API_KEY";
        let prev = std::env::var(var).ok();
        std::env::remove_var(var);
        let cfg = SummarizeConfig {
            model: "openai/x".into(),
            api_key: Some("from-config".into()),
            base_url: None,
            max_comments: 200,
            system_prompt: String::new(),
        };
        assert_eq!(cfg.resolve_api_key().as_deref(), Some("from-config"));
        if let Some(v) = prev {
            std::env::set_var(var, v);
        }
    }

    #[test]
    fn resolve_api_key_returns_none_when_unset() {
        let var = "HNTUI_LLM_API_KEY";
        let prev = std::env::var(var).ok();
        std::env::remove_var(var);
        let cfg = SummarizeConfig {
            model: "openai/x".into(),
            api_key: None,
            base_url: None,
            max_comments: 200,
            system_prompt: String::new(),
        };
        assert!(cfg.resolve_api_key().is_none());
        if let Some(v) = prev {
            std::env::set_var(var, v);
        }
    }
}
