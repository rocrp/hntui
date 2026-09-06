use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputLayer {
    Help,
    Summary,
    Article,
    FeedFilter,
    FilterText,
    SearchText,
    View,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpAction {
    Dismiss,
    ScrollDown(usize),
    ScrollUp(usize),
    PageDown,
    PageUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryAction {
    Dismiss,
    ScrollDown(usize),
    ScrollUp(usize),
    PageDown,
    PageUp,
    GoTop,
    GoBottom,
    Copy,
    OpenHelp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArticleAction {
    Dismiss,
    ScrollDown(usize),
    ScrollUp(usize),
    PageDown,
    PageUp,
    GoTop,
    GoBottom,
    Copy,
    OpenBrowser,
    SelectNextLink,
    SelectPreviousLink,
    OpenSelectedLink,
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
/// The filter and search inputs are one-line fields; cursor movement and word
/// deletion left with the settings popup that needed them.
pub enum TextAction {
    Submit,
    Cancel,
    Insert(char),
    DeleteBackward,
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
mod routing_tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Noop,
    Help(HelpAction),
    Summary(SummaryAction),
    Article(ArticleAction),
    FeedFilter(FeedFilterAction),
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
    /// Hand the Story off to another agent. Reachable from every layer that
    /// shows one, so it is a top-level Action rather than a per-layer one.
    Handoff,
    Refresh,
    Summarize,
    ViewArticle,
    StartSearch,
    OpenFeedFilter,
    OpenFilter,
    EditConfig,
    CopyComment,
    SelectStory(usize),
    SelectComment(usize),
}

#[derive(Debug, Default)]
pub struct KeyState {
    pending_g_layer: Option<InputLayer>,
}

impl KeyState {
    pub fn on_key(&mut self, layer: InputLayer, key: KeyEvent) -> Action {
        if !matches!(
            layer,
            InputLayer::Summary | InputLayer::Article | InputLayer::View
        ) {
            self.pending_g_layer = None;
        }
        match layer {
            InputLayer::Help => match (key.code, key.modifiers) {
                (KeyCode::Char('?'), _)
                | (KeyCode::Esc, _)
                | (KeyCode::Char('q'), KeyModifiers::NONE)
                | (KeyCode::Char('c'), KeyModifiers::CONTROL) => Action::Help(HelpAction::Dismiss),
                (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
                    Action::Help(HelpAction::ScrollDown(1))
                }
                (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => {
                    Action::Help(HelpAction::ScrollUp(1))
                }
                (KeyCode::Char('d'), KeyModifiers::CONTROL) => Action::Help(HelpAction::PageDown),
                (KeyCode::Char('u'), KeyModifiers::CONTROL) => Action::Help(HelpAction::PageUp),
                _ => Action::Noop,
            },
            InputLayer::Summary => self.summary_action(key),
            InputLayer::Article => self.article_action(key),
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
            InputLayer::FilterText => text_action(key)
                .map(Action::FilterInput)
                .unwrap_or(Action::Noop),
            InputLayer::SearchText => text_action(key)
                .map(Action::SearchInput)
                .unwrap_or(Action::Noop),
            InputLayer::View => self.view_action(key),
        }
    }

    fn summary_action(&mut self, key: KeyEvent) -> Action {
        if let Some(action) = self.g_sequence_action(
            InputLayer::Summary,
            key,
            Action::Summary(SummaryAction::GoTop),
        ) {
            return action;
        }

        match (key.code, key.modifiers) {
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
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => Action::Summary(SummaryAction::PageDown),
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => Action::Summary(SummaryAction::PageUp),
            (KeyCode::Char('G'), KeyModifiers::SHIFT)
            | (KeyCode::Char('G'), KeyModifiers::NONE) => Action::Summary(SummaryAction::GoBottom),
            (KeyCode::Char('c'), KeyModifiers::NONE) => Action::Summary(SummaryAction::Copy),
            (KeyCode::Char('H'), _) => Action::Handoff,
            _ => Action::Noop,
        }
    }

    fn article_action(&mut self, key: KeyEvent) -> Action {
        if let Some(action) = self.g_sequence_action(
            InputLayer::Article,
            key,
            Action::Article(ArticleAction::GoTop),
        ) {
            return action;
        }

        match (key.code, key.modifiers) {
            (KeyCode::Esc, _)
            | (KeyCode::Char('q'), KeyModifiers::NONE)
            | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                Action::Article(ArticleAction::Dismiss)
            }
            (KeyCode::Char('?'), _) => Action::Article(ArticleAction::OpenHelp),
            (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
                Action::Article(ArticleAction::ScrollDown(1))
            }
            (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => {
                Action::Article(ArticleAction::ScrollUp(1))
            }
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => Action::Article(ArticleAction::PageDown),
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => Action::Article(ArticleAction::PageUp),
            (KeyCode::PageDown, _) => Action::Article(ArticleAction::PageDown),
            (KeyCode::PageUp, _) => Action::Article(ArticleAction::PageUp),
            (KeyCode::Char('G'), KeyModifiers::SHIFT)
            | (KeyCode::Char('G'), KeyModifiers::NONE) => Action::Article(ArticleAction::GoBottom),
            (KeyCode::Char('c'), KeyModifiers::NONE) => Action::Article(ArticleAction::Copy),
            (KeyCode::Char('H'), _) => Action::Handoff,
            (KeyCode::Char('o'), KeyModifiers::NONE) => Action::Article(ArticleAction::OpenBrowser),
            (KeyCode::BackTab, _) | (KeyCode::Tab, KeyModifiers::SHIFT) => {
                Action::Article(ArticleAction::SelectPreviousLink)
            }
            (KeyCode::Tab, KeyModifiers::NONE) => Action::Article(ArticleAction::SelectNextLink),
            (KeyCode::Enter, _) => Action::Article(ArticleAction::OpenSelectedLink),
            _ => Action::Noop,
        }
    }

    fn view_action(&mut self, key: KeyEvent) -> Action {
        if let Some(action) = self.g_sequence_action(InputLayer::View, key, Action::GoTop) {
            return action;
        }

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
            (KeyCode::Char('v'), KeyModifiers::NONE) => Action::ViewArticle,
            (KeyCode::Char('/'), _) => Action::StartSearch,
            (KeyCode::Char('f'), KeyModifiers::NONE) => Action::OpenFeedFilter,
            (KeyCode::Char('F'), KeyModifiers::SHIFT)
            | (KeyCode::Char('F'), KeyModifiers::NONE) => Action::OpenFilter,
            (KeyCode::Char('y'), KeyModifiers::NONE) => Action::CopyComment,
            (KeyCode::Char('H'), _) => Action::Handoff,
            (KeyCode::Char(','), KeyModifiers::NONE) => Action::EditConfig,
            _ => Action::Noop,
        }
    }

    fn g_sequence_action(
        &mut self,
        layer: InputLayer,
        key: KeyEvent,
        go_top: Action,
    ) -> Option<Action> {
        if !matches!(
            (key.code, key.modifiers),
            (KeyCode::Char('g'), KeyModifiers::NONE)
        ) {
            self.pending_g_layer = None;
            return None;
        }

        if self.pending_g_layer == Some(layer) {
            self.pending_g_layer = None;
            return Some(go_top);
        }
        self.pending_g_layer = Some(layer);
        Some(Action::Noop)
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
