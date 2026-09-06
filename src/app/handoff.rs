use super::{App, AppEvent, TaskTarget, View};
use crate::api::Story;
use crate::handoff::{
    instruction_line, paste_filename, HandoffDocument, HandoffStatus, HandoffSummary, PasteRequest,
};
use crate::ui::summary_overlay::SummaryState;

impl App {
    /// `H`: publish what is loaded for this Story and put the instruction line
    /// on the clipboard. Nothing here fetches — a Handoff carries what the user
    /// has already seen, so it is refused rather than made to wait.
    pub(super) fn start_handoff(&mut self) {
        if self.tasks.is_running(TaskTarget::Handoff) {
            self.refuse_handoff("already in progress");
            return;
        }
        if matches!(
            self.summary_overlay.state(),
            SummaryState::Loading | SummaryState::Streaming
        ) {
            self.refuse_handoff("summary still streaming");
            return;
        }
        let Some(story) = self.handoff_story() else {
            self.refuse_handoff("no story selected");
            return;
        };
        let document = self.handoff_document(&story);
        if document.is_empty() {
            self.refuse_handoff("nothing loaded to hand off");
            return;
        }

        let request = PasteRequest {
            content: document.render(&story),
            filename: paste_filename(story.id),
        };
        let service = self.paste_service.clone();
        self.handoff_status = Some(HandoffStatus::InFlight);
        self.tasks.spawn(
            TaskTarget::Handoff,
            async move { service.create(request).await },
            |task, paste| AppEvent::HandoffCreated { task, paste },
        );
    }

    fn refuse_handoff(&mut self, reason: &str) {
        self.handoff_status = Some(HandoffStatus::Failed(reason.to_string()));
    }

    /// The Paste is made; all that is left is to hand its URL over. A clipboard
    /// that refuses it does not make this a failure — the URL is the deliverable.
    pub(super) fn finish_handoff(&mut self, raw_url: String) {
        let clipboard_error = self
            .clipboard
            .copy(&instruction_line(&raw_url))
            .err()
            .map(|error| format!("{error:#}"));
        self.handoff_status = Some(HandoffStatus::Done {
            raw_url,
            clipboard_error,
        });
    }

    /// Which Story a Handoff is about. An overlay is about the Story it was
    /// opened for, whatever the list has since been scrolled to.
    fn handoff_story(&self) -> Option<Story> {
        let overlay_story_id = if self.summary_overlay.is_visible() {
            Some(self.summary_overlay.story_id())
        } else if self.article_overlay.is_visible() {
            Some(self.article_overlay.story_id())
        } else {
            None
        };
        match overlay_story_id {
            Some(id) => self.story_by_id(id),
            None => match self.view {
                View::Stories => self.selected_story().cloned(),
                View::Comments => self.current_story.clone(),
            },
        }
    }

    fn story_by_id(&self, id: u64) -> Option<Story> {
        if let Some(story) = self.current_story.as_ref().filter(|story| story.id == id) {
            return Some(story.clone());
        }
        self.stories.iter().find(|story| story.id == id).cloned()
    }

    /// Everything loaded for this Story, and nothing else.
    fn handoff_document(&self, story: &Story) -> HandoffDocument {
        HandoffDocument {
            summary: self.handoff_summary(story),
            ..Default::default()
        }
    }

    fn handoff_summary(&self, story: &Story) -> Option<HandoffSummary> {
        if self.summary_overlay.state() != SummaryState::Done
            || self.summary_overlay.story_id() != story.id
        {
            return None;
        }
        Some(HandoffSummary {
            text: self.summary_overlay.summary_text().to_string(),
            model: self.summary_overlay.model_label(),
        })
    }
}
