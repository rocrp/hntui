use super::tests::{cli, key, left_click};
use super::*;
use crate::api::{InMemorySource, Sources};
use crate::config::Config;
use crate::input::Action;
use crate::summarizer::Summarizer;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use std::sync::Arc;

fn test_app() -> App {
    let source = Arc::new(InMemorySource::default());
    let sources = Sources::new(source.clone(), source);
    let (tx, _rx) = mpsc::unbounded_channel();
    let config = Config::for_test(std::env::temp_dir().join("hntui-test-config.toml"));
    let summarizer = Summarizer::new(None, None, reqwest::Client::new());
    App::new(cli(), sources, tx, None, config, summarizer)
}

#[test]
fn help_keys_scroll_through_actions_and_reopening_starts_at_the_top() {
    let mut app = test_app();
    app.prepare_frame(Rect::new(0, 0, 80, 10));
    app.handle_action(Action::OpenHelp);

    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
    assert_eq!(app.help_overlay.scroll_offset(), 6);

    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert_eq!(app.help_overlay.scroll_offset(), 0);

    app.handle_key(key(KeyCode::Char('j')));
    assert_eq!(app.help_overlay.scroll_offset(), 1);
    app.handle_key(key(KeyCode::Char('?')));
    app.handle_action(Action::OpenHelp);

    assert_eq!(app.help_overlay.scroll_offset(), 0);
}

#[test]
fn help_mouse_wheel_scrolls_while_a_left_click_dismisses() {
    let mut app = test_app();
    app.prepare_frame(Rect::new(0, 0, 80, 10));
    app.handle_action(Action::OpenHelp);

    app.handle_mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 1,
        row: 1,
        modifiers: KeyModifiers::NONE,
    });

    assert!(app.help_visible);
    assert_eq!(app.help_overlay.scroll_offset(), 3);

    app.handle_mouse(left_click(1, 1));

    assert!(!app.help_visible);
}
