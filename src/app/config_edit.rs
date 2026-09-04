//! ConfigReload — re-reading the config file after the Editor returns and
//! applying it to the running app.
//!
//! The App only ever asks for the Editor; the run loop owns the terminal, so it
//! performs the handoff and calls back here with the outcome.

use super::App;
use crate::config::Config;
use crate::editor::EditOutcome;
use std::time::Duration;

/// How a ConnectionTest against the reloaded config is going.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum ConnectionTestState {
    #[default]
    Idle,
    Testing,
    Success {
        model: String,
        ttft: Duration,
    },
    Error(String),
}

/// What the last ConfigReload did. Shown in the status line until the user
/// moves on to something else.
#[derive(Debug, Clone)]
pub struct ConfigStatus {
    /// Leading text: either the reload line or the reason it failed.
    pub message: String,
    /// Where the key came from and where requests will go. Dropped once the
    /// ConnectionTest settles: the status line is one row, and a test that
    /// actually reached the endpoint says more than the URL did.
    pub detail: Option<String>,
    pub failed: bool,
    pub test: ConnectionTestState,
}

impl ConfigStatus {
    fn failure(message: String) -> Self {
        Self {
            message,
            detail: None,
            failed: true,
            test: ConnectionTestState::Idle,
        }
    }
}

impl App {
    /// Records that the user asked to edit the config. The run loop picks this
    /// up, since it — not the App — owns the terminal.
    pub(super) fn request_editor(&mut self) {
        self.editor_requested = true;
    }

    /// Whether the run loop should hand the terminal to the Editor now.
    pub fn take_editor_request(&mut self) -> bool {
        std::mem::take(&mut self.editor_requested)
    }

    /// The file the Editor should open, created from the template first when it
    /// does not exist yet.
    pub fn config_path_for_editing(&mut self) -> std::path::PathBuf {
        if let Err(error) = self.config.ensure_file_exists() {
            self.last_error = Some(format!("config: {error:#}"));
        }
        self.config.path().to_path_buf()
    }

    /// Performs the ConfigReload once the Editor has exited.
    pub fn finish_config_edit(&mut self, outcome: anyhow::Result<EditOutcome>) {
        self.last_error = None;
        match outcome {
            Err(error) => {
                self.config_status =
                    Some(ConfigStatus::failure(format!("editor failed: {error:#}")));
            }
            Ok(EditOutcome::Cancelled { status }) => {
                self.config_status = Some(ConfigStatus::failure(format!(
                    "editor exited with status {status} · config not reloaded"
                )));
            }
            Ok(EditOutcome::Finished) => self.reload_config(),
        }
    }

    fn reload_config(&mut self) {
        let path = display_path(self.config.path());
        match self.config.reload() {
            Ok(config) => {
                let endpoint = resolved_endpoint(&config);
                self.apply_config(config);
                let mut detail: Vec<String> = Vec::new();
                if let Some(key_source) = self.config.effective_api_key().status() {
                    detail.push(key_source);
                }
                detail.extend(endpoint);
                let configured = self.config.summarize().is_some();
                self.config_status = Some(ConfigStatus {
                    message: format!("config reloaded · {path}"),
                    detail: (!detail.is_empty()).then(|| detail.join(" · ")),
                    failed: false,
                    test: ConnectionTestState::Idle,
                });
                // Editing the LLM settings is exactly the moment to find out
                // whether they work.
                if configured {
                    self.start_connection_test();
                }
            }
            // The running config stays in force: a typo must not take the
            // session down with it.
            Err(error) => {
                self.config_status = Some(ConfigStatus::failure(format!("{path}: {error:#}")));
            }
        }
    }

    /// Adopts a reloaded config everywhere it is read from.
    fn apply_config(&mut self, config: Config) {
        self.summarizer
            .update_config(config.summarize().cloned(), config.api_key_override());
        // The fetcher holds the configured binary, so a changed `[article].bin`
        // only takes effect if it is rebuilt.
        self.article_fetcher = self.article_fetcher.with_bin(config.article_bin());
        self.config = config;
    }
}

/// The POST URL the reloaded config resolves to, when it names a model at all.
fn resolved_endpoint(config: &Config) -> Option<String> {
    let summarize = config.summarize()?;
    let base_url = summarize.base_url.as_deref().filter(|url| !url.is_empty());
    match smolllm::resolve_endpoints(&summarize.model, base_url) {
        Ok(endpoints) => {
            let first = endpoints.first()?;
            let extra = endpoints.len() - 1;
            let suffix = if extra == 0 {
                String::new()
            } else {
                format!(" (+{extra} more)")
            };
            Some(format!("POST {}{suffix}", first.url))
        }
        Err(error) => Some(error.to_string()),
    }
}

/// Config paths live under `$HOME`; showing the whole thing wastes a status
/// line that is already tight.
fn display_path(path: &std::path::Path) -> String {
    let text = path.display().to_string();
    let Some(home) = std::env::var_os("HOME") else {
        return text;
    };
    let home = home.to_string_lossy();
    if home.is_empty() {
        return text;
    }
    match text.strip_prefix(home.as_ref()) {
        Some(rest) => format!("~{rest}"),
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ui::config_status_line;

    fn rendered(status: &ConfigStatus) -> String {
        config_status_line(status)
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn reloaded() -> ConfigStatus {
        ConfigStatus {
            message: "config reloaded · ~/.config/hntui/config.toml".to_string(),
            detail: Some(
                "set by HNTUI_LLM_API_KEY · POST https://gateway.example/v1/chat/completions"
                    .to_string(),
            ),
            failed: false,
            test: ConnectionTestState::Idle,
        }
    }

    #[test]
    fn the_endpoint_shows_until_a_real_request_has_been_down_that_path() {
        let mut status = reloaded();
        assert!(rendered(&status).contains("POST https://gateway.example"));

        status.test = ConnectionTestState::Testing;
        let testing = rendered(&status);
        assert!(
            testing.contains("POST https://gateway.example"),
            "{testing}"
        );
        assert!(testing.contains("⏳ testing"), "{testing}");

        status.test = ConnectionTestState::Success {
            model: "smolserver/summary → gpt-5!high".to_string(),
            ttft: Duration::from_millis(400),
        };
        let success = rendered(&status);
        assert_eq!(
            success,
            "config reloaded · ~/.config/hntui/config.toml · ✓ smolserver/summary → gpt-5!high · 400ms"
        );

        status.test = ConnectionTestState::Error("check API key".to_string());
        let failed = rendered(&status);
        assert_eq!(
            failed,
            "config reloaded · ~/.config/hntui/config.toml · ✗ check API key"
        );
    }

    #[test]
    fn a_failed_reload_says_only_what_went_wrong() {
        let status = ConfigStatus::failure("~/config.toml: parse error at line 2".to_string());

        assert!(status.failed);
        assert_eq!(rendered(&status), "~/config.toml: parse error at line 2");
    }

    #[test]
    fn a_home_path_is_shortened_for_the_status_line() {
        let home = std::env::var("HOME").expect("HOME is set in tests");
        let path = std::path::PathBuf::from(&home).join(".config/hntui/config.toml");

        assert_eq!(display_path(&path), "~/.config/hntui/config.toml");
        assert_eq!(
            display_path(std::path::Path::new("/etc/hntui.toml")),
            "/etc/hntui.toml"
        );
    }
}
