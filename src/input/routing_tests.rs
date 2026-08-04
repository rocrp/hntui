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
            InputLayer::Article,
            key(KeyCode::Esc),
            Action::Article(ArticleAction::Dismiss),
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

#[test]
fn enter_activates_the_selected_settings_row() {
    assert_eq!(
        KeyState::default().on_key(InputLayer::Settings, key(KeyCode::Enter)),
        Action::Settings(SettingsAction::Activate)
    );
}

#[test]
fn uppercase_g_routes_to_summary_bottom() {
    assert_eq!(
        KeyState::default().on_key(InputLayer::Summary, key(KeyCode::Char('G'))),
        Action::Summary(SummaryAction::GoBottom)
    );
}

#[test]
fn double_g_routes_to_summary_top() {
    let mut keys = KeyState::default();

    assert_eq!(
        keys.on_key(InputLayer::Summary, key(KeyCode::Char('g'))),
        Action::Noop
    );
    assert_eq!(
        keys.on_key(InputLayer::Summary, key(KeyCode::Char('g'))),
        Action::Summary(SummaryAction::GoTop)
    );
}

#[test]
fn non_g_summary_key_executes_normally_and_cancels_pending_g() {
    let mut keys = KeyState::default();

    assert_eq!(
        keys.on_key(InputLayer::Summary, key(KeyCode::Char('g'))),
        Action::Noop
    );
    assert_eq!(
        keys.on_key(InputLayer::Summary, key(KeyCode::Char('j'))),
        Action::Summary(SummaryAction::ScrollDown(1))
    );
    assert_eq!(
        keys.on_key(InputLayer::Summary, key(KeyCode::Char('g'))),
        Action::Noop
    );
    assert_eq!(
        keys.on_key(InputLayer::Summary, key(KeyCode::Char('g'))),
        Action::Summary(SummaryAction::GoTop)
    );
}

#[test]
fn pending_g_does_not_cross_input_layers() {
    let mut keys = KeyState::default();

    assert_eq!(
        keys.on_key(InputLayer::Summary, key(KeyCode::Char('g'))),
        Action::Noop
    );
    assert_eq!(
        keys.on_key(InputLayer::View, key(KeyCode::Char('g'))),
        Action::Noop
    );
    assert_eq!(
        keys.on_key(InputLayer::View, key(KeyCode::Char('g'))),
        Action::GoTop
    );
}

#[test]
fn v_opens_the_article_from_a_view_but_is_inert_inside_the_overlay() {
    let v = key(KeyCode::Char('v'));

    assert_eq!(
        KeyState::default().on_key(InputLayer::View, v),
        Action::ViewArticle
    );
    // Overlays never stack: `v` does nothing from inside one.
    assert_eq!(
        KeyState::default().on_key(InputLayer::Article, v),
        Action::Noop
    );
    assert_eq!(
        KeyState::default().on_key(InputLayer::Summary, v),
        Action::Noop
    );
}

#[test]
fn article_keys_route_to_article_actions() {
    let cases = [
        (
            key(KeyCode::Char('j')),
            Action::Article(ArticleAction::ScrollDown(1)),
        ),
        (
            key(KeyCode::Char('k')),
            Action::Article(ArticleAction::ScrollUp(1)),
        ),
        (
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
            Action::Article(ArticleAction::PageDown),
        ),
        (
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
            Action::Article(ArticleAction::PageUp),
        ),
        (
            key(KeyCode::PageDown),
            Action::Article(ArticleAction::PageDown),
        ),
        (key(KeyCode::PageUp), Action::Article(ArticleAction::PageUp)),
        (
            key(KeyCode::Char('G')),
            Action::Article(ArticleAction::GoBottom),
        ),
        (
            key(KeyCode::Char('c')),
            Action::Article(ArticleAction::Copy),
        ),
        (
            key(KeyCode::Char('o')),
            Action::Article(ArticleAction::OpenBrowser),
        ),
        (
            key(KeyCode::Tab),
            Action::Article(ArticleAction::SelectNextLink),
        ),
        (
            key(KeyCode::BackTab),
            Action::Article(ArticleAction::SelectPreviousLink),
        ),
        (
            key(KeyCode::Enter),
            Action::Article(ArticleAction::OpenSelectedLink),
        ),
        (
            key(KeyCode::Char('q')),
            Action::Article(ArticleAction::Dismiss),
        ),
        (
            key(KeyCode::Char('?')),
            Action::Article(ArticleAction::OpenHelp),
        ),
    ];

    for (key, expected) in cases {
        assert_eq!(
            KeyState::default().on_key(InputLayer::Article, key),
            expected
        );
    }
}

#[test]
fn double_g_routes_to_article_top_without_leaking_across_layers() {
    let mut keys = KeyState::default();

    assert_eq!(
        keys.on_key(InputLayer::Article, key(KeyCode::Char('g'))),
        Action::Noop
    );
    assert_eq!(
        keys.on_key(InputLayer::Article, key(KeyCode::Char('g'))),
        Action::Article(ArticleAction::GoTop)
    );

    assert_eq!(
        keys.on_key(InputLayer::Article, key(KeyCode::Char('g'))),
        Action::Noop
    );
    assert_eq!(
        keys.on_key(InputLayer::Summary, key(KeyCode::Char('g'))),
        Action::Noop
    );
    assert_eq!(
        keys.on_key(InputLayer::Summary, key(KeyCode::Char('g'))),
        Action::Summary(SummaryAction::GoTop)
    );
}

#[test]
fn help_scroll_keys_route_to_help_actions() {
    let cases = [
        (
            key(KeyCode::Char('j')),
            Action::Help(HelpAction::ScrollDown(1)),
        ),
        (key(KeyCode::Down), Action::Help(HelpAction::ScrollDown(1))),
        (
            key(KeyCode::Char('k')),
            Action::Help(HelpAction::ScrollUp(1)),
        ),
        (key(KeyCode::Up), Action::Help(HelpAction::ScrollUp(1))),
        (
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
            Action::Help(HelpAction::PageDown),
        ),
        (
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
            Action::Help(HelpAction::PageUp),
        ),
    ];

    for (key, expected) in cases {
        assert_eq!(KeyState::default().on_key(InputLayer::Help, key), expected);
    }
}
