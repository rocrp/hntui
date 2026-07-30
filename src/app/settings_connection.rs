use super::settings_popup::nonempty_owned;
use super::{App, AppEvent, ConnectionTestState, SettingsPopup, TaskId, TaskTarget};
use crate::config::default_system_prompt;
use crate::summarizer::{ConnectionDraft, ConnectionTestError, ConnectionTestSuccess};

impl App {
    pub(super) fn start_connection_test(&mut self) {
        let popup = self
            .settings_popup
            .as_ref()
            .expect("connection test without settings popup");
        let hntui_key = std::env::var("HNTUI_LLM_API_KEY").ok();
        let draft = connection_draft(popup, hntui_key.as_deref());
        self.settings_popup
            .as_mut()
            .expect("connection test without settings popup")
            .connection_test = ConnectionTestState::Testing;

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

    pub(super) fn cancel_connection_test(&mut self) {
        self.tasks.cancel(TaskTarget::ConnectionTest);
        if let Some(popup) = self.settings_popup.as_mut() {
            popup.connection_test = ConnectionTestState::Idle;
        }
    }

    pub(super) fn handle_connection_test_finished(
        &mut self,
        task: TaskId,
        result: Result<ConnectionTestSuccess, ConnectionTestError>,
    ) {
        if !self.tasks.finish(task) {
            return;
        }
        let Some(popup) = self.settings_popup.as_mut() else {
            return;
        };
        popup.connection_test = match result {
            Ok(success) => ConnectionTestState::Success {
                model: success.model,
                ttft: success.ttft,
            },
            Err(error) => ConnectionTestState::Error(error.friendly_message()),
        };
    }
}

fn connection_draft(popup: &SettingsPopup, hntui_key: Option<&str>) -> ConnectionDraft {
    let system_prompt = if popup.system_prompt.trim().is_empty() {
        default_system_prompt()
    } else {
        popup.system_prompt.clone()
    };
    ConnectionDraft {
        model: popup.model.trim().to_string(),
        system_prompt,
        api_key: resolve_draft_api_key(hntui_key, &popup.api_key),
        base_url: nonempty_owned(&popup.base_url),
    }
}

fn resolve_draft_api_key(hntui_key: Option<&str>, draft_key: &str) -> Option<String> {
    hntui_key
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .or_else(|| {
            let key = draft_key.trim();
            (!key.is_empty()).then_some(key)
        })
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{InMemorySource, Sources};
    use crate::config::Config;
    use crate::input::{Action, SettingsAction};
    use crate::summarizer::Summarizer;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    fn test_app_with_events() -> (App, mpsc::UnboundedReceiver<AppEvent>) {
        let source = Arc::new(InMemorySource::default());
        let (tx, rx) = mpsc::unbounded_channel();
        (
            App::new(
                super::super::tests::cli(),
                Sources::new(source.clone(), source),
                tx,
                None,
                Config::for_test(std::env::temp_dir().join("hntui-connection-test.toml")),
                Summarizer::new(None, None, reqwest::Client::new()),
                super::super::tests::test_article_fetcher(),
            ),
            rx,
        )
    }

    fn test_app() -> App {
        test_app_with_events().0
    }

    async fn next_connection_event(rx: &mut mpsc::UnboundedReceiver<AppEvent>) -> AppEvent {
        tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("connection event timed out")
            .expect("app event channel closed")
    }

    #[test]
    fn draft_api_key_precedence_matches_the_summarizer_chain() {
        let cases = [
            (
                Some(" hntui-key "),
                "draft-key",
                Some("hntui-key"),
                "HNTUI env shadows draft",
            ),
            (
                None,
                " draft-key ",
                Some("draft-key"),
                "draft plays the file role",
            ),
            (
                Some("   "),
                "draft-key",
                Some("draft-key"),
                "blank HNTUI env does not shadow draft",
            ),
            (None, "   ", None, "None delegates to the provider env"),
        ];

        for (hntui_key, draft_key, expected, reason) in cases {
            assert_eq!(
                resolve_draft_api_key(hntui_key, draft_key).as_deref(),
                expected,
                "{reason}"
            );
        }
    }

    #[test]
    fn connection_draft_uses_unsaved_popup_values() {
        let mut app = test_app();
        app.handle_action(Action::OpenSettings);
        let popup = app.settings_popup.as_mut().expect("settings");
        popup.model = " custom/draft-model ".to_string();
        popup.api_key = "draft-key".to_string();
        popup.base_url = " https://draft.example/full# ".to_string();
        popup.system_prompt = "Draft instructions".to_string();

        let draft = connection_draft(popup, None);

        assert_eq!(draft.model, "custom/draft-model");
        assert_eq!(draft.api_key.as_deref(), Some("draft-key"));
        assert_eq!(
            draft.base_url.as_deref(),
            Some("https://draft.example/full#")
        );
        assert_eq!(draft.system_prompt, "Draft instructions");
    }

    #[tokio::test]
    async fn activating_the_eighth_row_starts_a_connection_test() {
        let mut app = test_app();
        app.handle_action(Action::OpenSettings);
        app.settings_popup.as_mut().expect("settings").cursor = 7;

        app.handle_action(Action::Settings(SettingsAction::Activate));

        assert_eq!(
            app.settings_popup
                .as_ref()
                .expect("settings")
                .connection_test,
            ConnectionTestState::Testing
        );
        assert!(app.tasks.is_running(TaskTarget::ConnectionTest));
    }

    #[tokio::test]
    async fn closing_rejects_a_queued_connection_result() {
        let (mut app, mut rx) = test_app_with_events();
        app.handle_action(Action::OpenSettings);
        app.settings_popup.as_mut().expect("settings").cursor = 7;
        app.handle_action(Action::Settings(SettingsAction::Activate));
        let queued = next_connection_event(&mut rx).await;

        app.handle_action(Action::Settings(SettingsAction::CloseAndSave));
        app.handle_app_event(queued);

        assert!(app.settings_popup.is_none());
        assert!(!app.tasks.is_running(TaskTarget::ConnectionTest));
    }

    #[tokio::test]
    async fn beginning_an_edit_rejects_a_queued_connection_result_and_resets_idle() {
        let (mut app, mut rx) = test_app_with_events();
        app.handle_action(Action::OpenSettings);
        app.settings_popup.as_mut().expect("settings").cursor = 7;
        app.handle_action(Action::Settings(SettingsAction::Activate));
        let queued = next_connection_event(&mut rx).await;

        app.settings_popup.as_mut().expect("settings").cursor = 0;
        app.handle_action(Action::Settings(SettingsAction::Activate));
        app.handle_app_event(queued);

        let popup = app.settings_popup.as_ref().expect("settings");
        assert!(popup.editing);
        assert_eq!(popup.connection_test, ConnectionTestState::Idle);
        assert!(!app.tasks.is_running(TaskTarget::ConnectionTest));
    }

    #[tokio::test]
    async fn a_new_connection_test_supersedes_a_queued_result() {
        let (mut app, mut rx) = test_app_with_events();
        app.handle_action(Action::OpenSettings);
        app.settings_popup.as_mut().expect("settings").cursor = 7;
        app.handle_action(Action::Settings(SettingsAction::Activate));
        let first = next_connection_event(&mut rx).await;
        app.handle_action(Action::Settings(SettingsAction::Activate));
        let second = next_connection_event(&mut rx).await;

        app.handle_app_event(first);
        assert_eq!(
            app.settings_popup
                .as_ref()
                .expect("settings")
                .connection_test,
            ConnectionTestState::Testing
        );
        app.handle_app_event(second);
        assert!(matches!(
            app.settings_popup
                .as_ref()
                .expect("settings")
                .connection_test,
            ConnectionTestState::Error(_)
        ));
    }
}
