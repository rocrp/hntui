use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    MoveDown,
    MoveUp,
    PageDown,
    PageUp,
    GoTop,
    GoBottom,
    OpenComments,
    OpenInBrowser,
    BackOrQuit,
    ToggleCollapse,
    Refresh,
}

#[derive(Debug, Default)]
pub struct KeyState {
    pending_g: bool,
}

impl KeyState {
    pub fn on_key(&mut self, key: KeyEvent) -> Option<Action> {
        match (key.code, key.modifiers) {
            (KeyCode::Char('g'), KeyModifiers::NONE) => {
                if self.pending_g {
                    self.pending_g = false;
                    Some(Action::GoTop)
                } else {
                    self.pending_g = true;
                    None
                }
            }
            _ => {
                self.pending_g = false;
                match (key.code, key.modifiers) {
                    (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
                        Some(Action::MoveDown)
                    }
                    (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => {
                        Some(Action::MoveUp)
                    }
                    (KeyCode::Char('d'), KeyModifiers::CONTROL) => Some(Action::PageDown),
                    (KeyCode::Char('u'), KeyModifiers::CONTROL) => Some(Action::PageUp),
                    (KeyCode::Char('G'), KeyModifiers::SHIFT)
                    | (KeyCode::Char('G'), KeyModifiers::NONE) => Some(Action::GoBottom),
                    (KeyCode::Enter, _) => Some(Action::OpenComments),
                    (KeyCode::Char('o'), KeyModifiers::NONE) => Some(Action::OpenInBrowser),
                    (KeyCode::Char('q'), KeyModifiers::NONE) | (KeyCode::Esc, _) => {
                        Some(Action::BackOrQuit)
                    }
                    (KeyCode::Char('c'), KeyModifiers::NONE) => Some(Action::ToggleCollapse),
                    (KeyCode::Char('r'), KeyModifiers::NONE) => Some(Action::Refresh),
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => Some(Action::BackOrQuit),
                    _ => None,
                }
            }
        }
    }
}
