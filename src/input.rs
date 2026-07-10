use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputLayer {
    Help,
    Summary,
    SettingsEditor,
    Settings,
    FeedFilter,
    FilterText,
    SearchText,
    View,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpAction {
    Dismiss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryAction {
    Dismiss,
    ScrollDown(usize),
    ScrollUp(usize),
    PageDown,
    PageUp,
    Copy,
    OpenHelp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedFilterAction {
    Dismiss,
    Select,
    SelectIndex(usize),
    MoveDown,
    MoveUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsAction {
    MoveDown,
    MoveUp,
    StartEditing,
    CloseAndSave,
    Edit(TextAction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAction {
    Submit,
    Cancel,
    Insert(char),
    DeleteBackward,
    DeleteForward,
    DeleteWordBackward,
    DeleteToStart,
    DeleteToEnd,
    MoveLeft,
    MoveRight,
    MoveWordLeft,
    MoveWordRight,
    MoveToStart,
    MoveToEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CursorStep {
    Previous,
    Next,
}

pub(crate) fn step_bounded(cursor: &mut usize, step: CursorStep, count: usize) {
    assert!(count > 0, "bounded cursor requires at least one item");
    *cursor = match step {
        CursorStep::Previous => cursor.saturating_sub(1),
        CursorStep::Next => (*cursor + 1).min(count - 1),
    };
}

#[cfg(test)]
mod routing_tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn same_key_routes_by_the_single_active_layer() {
        let cases = [
            (InputLayer::View, key(KeyCode::Esc), Action::BackOrQuit),
            (
                InputLayer::Help,
                key(KeyCode::Esc),
                Action::Help(HelpAction::Dismiss),
            ),
            (
                InputLayer::Summary,
                key(KeyCode::Esc),
                Action::Summary(SummaryAction::Dismiss),
            ),
            (
                InputLayer::FeedFilter,
                key(KeyCode::Esc),
                Action::FeedFilter(FeedFilterAction::Dismiss),
            ),
            (
                InputLayer::Settings,
                key(KeyCode::Esc),
                Action::Settings(SettingsAction::CloseAndSave),
            ),
            (
                InputLayer::FilterText,
                key(KeyCode::Esc),
                Action::FilterInput(TextAction::Cancel),
            ),
            (
                InputLayer::SearchText,
                key(KeyCode::Esc),
                Action::SearchInput(TextAction::Cancel),
            ),
        ];

        for (layer, key, expected) in cases {
            assert_eq!(KeyState::default().on_key(layer, key), expected);
        }
    }

    #[test]
    fn question_mark_is_help_in_view_but_text_in_an_input() {
        let question = key(KeyCode::Char('?'));

        assert_eq!(
            KeyState::default().on_key(InputLayer::View, question),
            Action::OpenHelp
        );
        assert_eq!(
            KeyState::default().on_key(InputLayer::SearchText, question),
            Action::SearchInput(TextAction::Insert('?'))
        );
    }

    #[test]
    fn unicode_text_editing_is_an_action() {
        assert_eq!(
            KeyState::default().on_key(InputLayer::SettingsEditor, key(KeyCode::Char('界')),),
            Action::Settings(SettingsAction::Edit(TextAction::Insert('界')))
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Noop,
    Help(HelpAction),
    Summary(SummaryAction),
    FeedFilter(FeedFilterAction),
    Settings(SettingsAction),
    FilterInput(TextAction),
    SearchInput(TextAction),
    MoveDown,
    MoveUp,
    PageDown,
    PageUp,
    GoTop,
    GoBottom,
    OpenHelp,
    Enter,
    OpenComments,
    OpenPrimaryBrowser,
    OpenSecondaryBrowser,
    BackOrQuit,
    Collapse,
    Expand,
    ToggleCollapse,
    Refresh,
    Summarize,
    StartSearch,
    OpenFeedFilter,
    OpenFilter,
    OpenSettings,
    CopyComment,
    SelectStory(usize),
    SelectComment(usize),
}

#[derive(Debug, Default)]
pub struct KeyState {
    pending_g: bool,
}

impl KeyState {
    pub fn on_key(&mut self, layer: InputLayer, key: KeyEvent) -> Action {
        if layer != InputLayer::View {
            self.pending_g = false;
        }
        match layer {
            InputLayer::Help => match (key.code, key.modifiers) {
                (KeyCode::Char('?'), _)
                | (KeyCode::Esc, _)
                | (KeyCode::Char('q'), KeyModifiers::NONE)
                | (KeyCode::Char('c'), KeyModifiers::CONTROL) => Action::Help(HelpAction::Dismiss),
                _ => Action::Noop,
            },
            InputLayer::Summary => match (key.code, key.modifiers) {
                (KeyCode::Esc, _)
                | (KeyCode::Char('q'), KeyModifiers::NONE)
                | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    Action::Summary(SummaryAction::Dismiss)
                }
                (KeyCode::Char('?'), _) => Action::Summary(SummaryAction::OpenHelp),
                (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
                    Action::Summary(SummaryAction::ScrollDown(1))
                }
                (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => {
                    Action::Summary(SummaryAction::ScrollUp(1))
                }
                (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                    Action::Summary(SummaryAction::PageDown)
                }
                (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                    Action::Summary(SummaryAction::PageUp)
                }
                (KeyCode::Char('c'), KeyModifiers::NONE) => Action::Summary(SummaryAction::Copy),
                _ => Action::Noop,
            },
            InputLayer::FeedFilter => match (key.code, key.modifiers) {
                (KeyCode::Esc, _) => Action::FeedFilter(FeedFilterAction::Dismiss),
                (KeyCode::Enter, _) => Action::FeedFilter(FeedFilterAction::Select),
                (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
                    Action::FeedFilter(FeedFilterAction::MoveDown)
                }
                (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => {
                    Action::FeedFilter(FeedFilterAction::MoveUp)
                }
                _ => Action::Noop,
            },
            InputLayer::Settings => match (key.code, key.modifiers) {
                (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
                    Action::Settings(SettingsAction::MoveDown)
                }
                (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => {
                    Action::Settings(SettingsAction::MoveUp)
                }
                (KeyCode::Enter, _) => Action::Settings(SettingsAction::StartEditing),
                (KeyCode::Esc, _) | (KeyCode::Char('q'), KeyModifiers::NONE) => {
                    Action::Settings(SettingsAction::CloseAndSave)
                }
                _ => Action::Noop,
            },
            InputLayer::SettingsEditor => settings_text_action(key)
                .map(|action| Action::Settings(SettingsAction::Edit(action)))
                .unwrap_or(Action::Noop),
            InputLayer::FilterText => text_action(key)
                .map(Action::FilterInput)
                .unwrap_or(Action::Noop),
            InputLayer::SearchText => text_action(key)
                .map(Action::SearchInput)
                .unwrap_or(Action::Noop),
            InputLayer::View => self.view_action(key),
        }
    }

    fn view_action(&mut self, key: KeyEvent) -> Action {
        if matches!(
            (key.code, key.modifiers),
            (KeyCode::Char('g'), KeyModifiers::NONE)
        ) {
            if self.pending_g {
                self.pending_g = false;
                return Action::GoTop;
            }
            self.pending_g = true;
            return Action::Noop;
        }

        self.pending_g = false;
        match (key.code, key.modifiers) {
            (KeyCode::Char('?'), _) => Action::OpenHelp,
            (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => Action::MoveDown,
            (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => Action::MoveUp,
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => Action::PageDown,
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => Action::PageUp,
            (KeyCode::Char('G'), KeyModifiers::SHIFT)
            | (KeyCode::Char('G'), KeyModifiers::NONE) => Action::GoBottom,
            (KeyCode::Enter, _) => Action::Enter,
            (KeyCode::Char(' '), KeyModifiers::NONE) => Action::OpenComments,
            (KeyCode::Char('o'), KeyModifiers::NONE) => Action::OpenPrimaryBrowser,
            (KeyCode::Char('o'), KeyModifiers::SHIFT) | (KeyCode::Char('O'), _) => {
                Action::OpenSecondaryBrowser
            }
            (KeyCode::Char('q'), KeyModifiers::NONE)
            | (KeyCode::Esc, _)
            | (KeyCode::Char('c'), KeyModifiers::CONTROL) => Action::BackOrQuit,
            (KeyCode::Char('h'), KeyModifiers::NONE) | (KeyCode::Left, _) => Action::Collapse,
            (KeyCode::Char('l'), KeyModifiers::NONE) | (KeyCode::Right, _) => Action::Expand,
            (KeyCode::Char('c'), KeyModifiers::NONE) => Action::ToggleCollapse,
            (KeyCode::Char('r'), KeyModifiers::NONE) => Action::Refresh,
            (KeyCode::Char('s'), KeyModifiers::NONE) => Action::Summarize,
            (KeyCode::Char('/'), _) => Action::StartSearch,
            (KeyCode::Char('f'), KeyModifiers::NONE) => Action::OpenFeedFilter,
            (KeyCode::Char('F'), KeyModifiers::SHIFT)
            | (KeyCode::Char('F'), KeyModifiers::NONE) => Action::OpenFilter,
            (KeyCode::Char('y'), KeyModifiers::NONE) => Action::CopyComment,
            (KeyCode::Char(','), KeyModifiers::NONE) => Action::OpenSettings,
            _ => Action::Noop,
        }
    }
}

fn text_action(key: KeyEvent) -> Option<TextAction> {
    match (key.code, key.modifiers) {
        (KeyCode::Enter, _) => Some(TextAction::Submit),
        (KeyCode::Esc, _) => Some(TextAction::Cancel),
        (KeyCode::Backspace, _) => Some(TextAction::DeleteBackward),
        (KeyCode::Char(character), modifiers)
            if modifiers == KeyModifiers::NONE || modifiers == KeyModifiers::SHIFT =>
        {
            Some(TextAction::Insert(character))
        }
        _ => None,
    }
}

fn settings_text_action(key: KeyEvent) -> Option<TextAction> {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Enter => Some(TextAction::Submit),
        KeyCode::Esc => Some(TextAction::Cancel),
        KeyCode::Left if alt => Some(TextAction::MoveWordLeft),
        KeyCode::Left => Some(TextAction::MoveLeft),
        KeyCode::Right if alt => Some(TextAction::MoveWordRight),
        KeyCode::Right => Some(TextAction::MoveRight),
        KeyCode::Home => Some(TextAction::MoveToStart),
        KeyCode::End => Some(TextAction::MoveToEnd),
        KeyCode::Char('a') if control => Some(TextAction::MoveToStart),
        KeyCode::Char('e') if control => Some(TextAction::MoveToEnd),
        KeyCode::Backspace if control || alt => Some(TextAction::DeleteWordBackward),
        KeyCode::Backspace => Some(TextAction::DeleteBackward),
        KeyCode::Delete => Some(TextAction::DeleteForward),
        KeyCode::Char('w') if control => Some(TextAction::DeleteWordBackward),
        KeyCode::Char('u') if control => Some(TextAction::DeleteToStart),
        KeyCode::Char('k') if control => Some(TextAction::DeleteToEnd),
        KeyCode::Char(character) if !control && !alt => Some(TextAction::Insert(character)),
        _ => None,
    }
}
