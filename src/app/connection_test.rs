//! ConnectionTest — verifying the LLM configuration just reloaded by sending a
//! minimal real request along the Summarizer's exact path.

use super::config_edit::ConnectionTestState;
use super::{App, AppEvent, TaskId, TaskTarget};
use crate::config::default_system_prompt;
use crate::summarizer::{ConnectionDraft, ConnectionTestError, ConnectionTestSuccess};

impl App {
    /// Tests the configuration now in force. Runs against the saved Config, so
    /// the key it uses is exactly the one the Summarizer will use.
    pub(super) fn start_connection_test(&mut self) {
        let Some(draft) = connection_draft(&self.config) else {
            return;
        };
        self.tasks.cancel(TaskTarget::ConnectionTest);
        self.set_connection_test_state(ConnectionTestState::Testing);

        let future = self.summarizer.test_connection(draft);
        self.tasks.spawn(
            TaskTarget::ConnectionTest,
            async move {
                Ok::<Result<ConnectionTestSuccess, ConnectionTestError>, anyhow::Error>(
                    future.await,
                )
            },
            |task, result| AppEvent::ConnectionTestFinished { task, result },
        );
    }

    pub(super) fn handle_connection_test_finished(
        &mut self,
        task: TaskId,
        result: Result<ConnectionTestSuccess, ConnectionTestError>,
    ) {
        if !self.tasks.finish(task) {
            return;
        }
        self.set_connection_test_state(match result {
            Ok(success) => ConnectionTestState::Success {
                model: success.model_label(),
                ttft: success.ttft,
            },
            Err(error) => ConnectionTestState::Error(error.friendly_message()),
        });
    }

    /// Only reports into a reload line that is still on screen; once the user
    /// has moved on, a late result has nowhere to go.
    fn set_connection_test_state(&mut self, state: ConnectionTestState) {
        if let Some(status) = self.config_status.as_mut() {
            status.test = state;
        }
    }
}

/// The draft to test: the configuration exactly as the Summarizer reads it.
fn connection_draft(config: &crate::config::Config) -> Option<ConnectionDraft> {
    let summarize = config.summarize()?;
    let system_prompt = if summarize.system_prompt.trim().is_empty() {
        default_system_prompt()
    } else {
        summarize.system_prompt.clone()
    };
    Some(ConnectionDraft {
        model: summarize.model.trim().to_string(),
        system_prompt,
        api_key: config.api_key_override(),
        base_url: summarize
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(str::to_string),
    })
}
