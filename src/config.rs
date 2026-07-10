use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Config {
    stored: StoredConfig,
    path: PathBuf,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct StoredConfig {
    summarize: Option<SummarizeConfig>,
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
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
}

pub struct ConfigEdits {
    pub summarize: SummarizeConfig,
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

fn default_max_comments() -> usize {
    200
}

pub(crate) fn default_system_prompt() -> String {
    "Summarize this Hacker News discussion concisely. \
     Highlight key arguments, disagreements, and consensus points."
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
        let stored = if path.exists() {
            let contents = std::fs::read_to_string(&path)
                .with_context(|| format!("read config {}", path.display()))?;
            toml::from_str(&contents).with_context(|| format!("parse config {}", path.display()))?
        } else {
            StoredConfig::default()
        };
        Ok(Self { stored, path })
    }

    #[cfg(test)]
    fn path(&self) -> &Path {
        &self.path
    }

    pub fn summarize(&self) -> Option<&SummarizeConfig> {
        self.stored.summarize.as_ref()
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

    pub async fn save(&self, edits: ConfigEdits) -> Result<Self> {
        let next = Self {
            stored: StoredConfig {
                summarize: Some(edits.summarize),
            },
            path: self.path.clone(),
        };
        let contents = toml::to_string_pretty(&next.stored).context("serialize config")?;
        if let Some(parent) = next
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("create config dir {}", parent.display()))?;
        }
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .context("system time")?
            .as_nanos();
        let temporary = next
            .path
            .with_extension(format!("toml.tmp.{}.{unique}", std::process::id()));
        tokio::fs::write(&temporary, contents)
            .await
            .with_context(|| format!("write temp config {}", temporary.display()))?;
        tokio::fs::rename(&temporary, &next.path)
            .await
            .with_context(|| {
                format!(
                    "rename config {} -> {}",
                    temporary.display(),
                    next.path.display()
                )
            })?;
        Ok(next)
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
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvGuard {
        name: &'static str,
        previous: Option<String>,
    }

    impl EnvGuard {
        fn set(name: &'static str, value: &str) -> Self {
            let previous = std::env::var(name).ok();
            std::env::set_var(name, value);
            Self { name, previous }
        }

        fn unset(name: &'static str) -> Self {
            let previous = std::env::var(name).ok();
            std::env::remove_var(name);
            Self { name, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                std::env::set_var(self.name, previous);
            } else {
                std::env::remove_var(self.name);
            }
        }
    }

    #[test]
    fn discovery_uses_first_existing_candidate() {
        let dir = tempfile::tempdir().expect("temp dir");
        let first = dir.path().join("config.toml");
        let legacy = dir.path().join("plugin-config.toml");
        std::fs::write(&first, "[summarize]\nmodel = \"openai/first\"\n")
            .expect("write first config");
        std::fs::write(&legacy, "[summarize]\nmodel = \"openai/legacy\"\n")
            .expect("write legacy config");

        let config =
            Config::load_from(vec![first.clone(), legacy], dir.path().join("default.toml"))
                .expect("load config");

        assert_eq!(config.path(), first);
        assert_eq!(
            config.summarize().expect("summarize config").model,
            "openai/first"
        );
    }

    #[test]
    fn canonical_discovery_candidates_precede_all_legacy_candidates() {
        let candidates = discovery_candidates();
        let legacy_start = candidates
            .iter()
            .position(|path| {
                path.file_name()
                    .is_some_and(|name| name == "plugin-config.toml")
            })
            .expect("legacy candidates present");

        assert!(legacy_start > 0);
        assert!(candidates[..legacy_start]
            .iter()
            .all(|path| path.file_name().is_some_and(|name| name == "config.toml")));
        assert!(candidates[legacy_start..].iter().all(|path| path
            .file_name()
            .is_some_and(|name| name == "plugin-config.toml")));
    }

    #[tokio::test]
    async fn save_round_trips_to_the_loaded_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let preferred = dir.path().join("config.toml");
        let legacy = dir.path().join("plugin-config.toml");
        let canonical = dir.path().join("canonical/config.toml");
        std::fs::write(&legacy, "[summarize]\nmodel = \"openai/old\"\n")
            .expect("write legacy config");
        let config = Config::load_from(vec![preferred.clone(), legacy.clone()], canonical.clone())
            .expect("load legacy config");

        let saved = config
            .save(ConfigEdits {
                summarize: SummarizeConfig {
                    model: "openai/new".to_string(),
                    api_key: None,
                    base_url: None,
                    max_comments: 50,
                    system_prompt: "Be terse".to_string(),
                },
            })
            .await
            .expect("save config");
        let reloaded =
            Config::load_from(vec![preferred, legacy.clone()], canonical).expect("reload config");

        assert_eq!(saved.path(), legacy);
        assert_eq!(
            reloaded.summarize().expect("summarize config").model,
            "openai/new"
        );
        assert!(!reloaded.path().ends_with("canonical/config.toml"));
    }

    #[test]
    fn hntui_env_key_reports_that_it_shadows_the_file() {
        let _lock = env_lock().lock().expect("env lock poisoned");
        let _env = EnvGuard::set("HNTUI_LLM_API_KEY", "from-env");
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[summarize]\nmodel = \"openai/test\"\napi_key = \"from-file\"\n",
        )
        .expect("write config");
        let config = Config::load_from(vec![path.clone()], path).expect("load config");

        let effective = config.effective_api_key();

        assert_eq!(effective.value.as_deref(), Some("from-env"));
        assert_eq!(effective.source, ValueSource::HntuiEnv);
        assert!(effective.file_shadowed);
    }

    #[test]
    fn file_key_precedes_provider_env_without_becoming_a_provider_override() {
        let _lock = env_lock().lock().expect("env lock poisoned");
        let _hntui = EnvGuard::unset("HNTUI_LLM_API_KEY");
        let _provider = EnvGuard::set("OPENAI_API_KEY", "from-provider");
        let dir = tempfile::tempdir().expect("temp dir");
        let with_key = dir.path().join("with-key.toml");
        let provider_only = dir.path().join("provider-only.toml");
        std::fs::write(
            &with_key,
            "[summarize]\nmodel = \"openai/test\"\napi_key = \"from-file\"\n",
        )
        .expect("write file-key config");
        std::fs::write(&provider_only, "[summarize]\nmodel = \"openai/test\"\n")
            .expect("write provider config");

        let file_config =
            Config::load_from(vec![with_key.clone()], with_key).expect("load file config");
        let provider_config = Config::load_from(vec![provider_only.clone()], provider_only)
            .expect("load provider config");

        assert!(matches!(
            file_config.effective_api_key().source,
            ValueSource::File(_)
        ));
        assert_eq!(file_config.api_key_override().as_deref(), Some("from-file"));
        assert_eq!(
            provider_config.effective_api_key().source,
            ValueSource::ProviderEnv("OPENAI_API_KEY".to_string())
        );
        assert_eq!(provider_config.api_key_override(), None);
    }

    #[tokio::test]
    async fn save_without_an_existing_file_uses_the_canonical_path() {
        let dir = tempfile::tempdir().expect("temp dir");
        let canonical = dir.path().join("hntui/config.toml");
        let config = Config::load_from(vec![dir.path().join("missing.toml")], canonical.clone())
            .expect("load empty config");

        config
            .save(ConfigEdits {
                summarize: SummarizeConfig {
                    model: "gemini/test".to_string(),
                    api_key: None,
                    base_url: None,
                    max_comments: 200,
                    system_prompt: "Summarize".to_string(),
                },
            })
            .await
            .expect("save canonical config");

        assert!(canonical.exists());
    }
}
