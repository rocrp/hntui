use super::*;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;
use std::time::Instant;

fn make_popup(model: &str, base_url: &str) -> SettingsPopup {
    SettingsPopup {
        cursor: 0,
        editing: false,
        edit_buffer: String::new(),
        edit_cursor: 0,
        model: model.to_string(),
        api_key: String::new(),
        base_url: base_url.to_string(),
        max_comments: "200".to_string(),
        include_article: "true".to_string(),
        max_article_chars: "20000".to_string(),
        system_prompt: String::new(),
        api_key_status: None,
        connection_test: crate::app::ConnectionTestState::Idle,
        dirty: false,
        saved_at: None,
    }
}

fn render_test_popup(popup: &SettingsPopup, width: u16, height: u16) -> (Buffer, Rect) {
    let area = Rect::new(0, 0, width, height);
    let popup_area = popup_rect(area, popup).expect("test terminal should fit settings popup");
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("create test terminal");
    terminal
        .draw(|frame| render_popup(frame, popup))
        .expect("render settings popup");
    (terminal.backend().buffer().clone(), popup_area)
}

fn find_text(buffer: &Buffer, area: Rect, needle: &str, min_row: u16) -> (u16, u16) {
    for row in min_row..area.bottom() {
        let mut line = String::new();
        let mut cell_starts = Vec::with_capacity(usize::from(area.width));
        for column in area.left()..area.right() {
            cell_starts.push((line.len(), column));
            line.push_str(buffer[(column, row)].symbol());
        }
        if let Some(byte_offset) = line.find(needle) {
            let column = cell_starts
                .iter()
                .rev()
                .find_map(|(start, column)| (*start <= byte_offset).then_some(*column))
                .expect("matched text has a buffer cell");
            return (column, row);
        }
    }
    panic!("{needle:?} not rendered in {area:?}");
}

#[test]
fn resolved_endpoint_preview_uses_the_wider_popup_and_model_count_suffix() {
    let popup = make_popup(
        "custom/first, custom/second, custom/third",
        "https://gateway.example",
    );

    let (buffer, area) = render_test_popup(&popup, 120, 30);

    assert_eq!(area.width, 100);
    find_text(
        &buffer,
        area,
        "POST https://gateway.example/v1/chat/completions (+2 more)",
        area.top(),
    );
}

#[test]
fn resolution_failure_is_rendered_inline_with_warn_style() {
    let popup = make_popup("hntui-issue-25-render-unknown/model", "");

    let (buffer, area) = render_test_popup(&popup, 120, 30);
    let (column, row) = find_text(&buffer, area, "missing base URL for provider", area.top());

    assert_eq!(buffer[(column, row)].fg, theme::PEACH);
    let (_, action_row) = find_text(&buffer, area, "Pass base_url or set", row);
    find_text(
        &buffer,
        area,
        "HNTUI_ISSUE_25_RENDER_UNKNOWN_BASE_URL",
        action_row,
    );
}

#[test]
fn long_resolved_endpoint_wraps_and_keeps_the_footer_visible() {
    let base_url = format!("https://gateway.example/{}END#", "x".repeat(100));
    let popup = make_popup("custom/model", &base_url);
    let short = make_popup("custom/model", "https://gateway.example");
    let terminal = Rect::new(0, 0, 70, 40);

    let short_area = popup_rect(terminal, &short).expect("short popup");
    let (buffer, long_area) = render_test_popup(&popup, terminal.width, terminal.height);
    let (_, post_row) = find_text(&buffer, long_area, "POST", long_area.top());
    let (_, endpoint_start_row) =
        find_text(&buffer, long_area, "https://gateway.example/", post_row);
    let (_, endpoint_end_row) = find_text(&buffer, long_area, "END", endpoint_start_row);
    let (_, footer_row) = find_text(&buffer, long_area, "j/k:nav", endpoint_end_row);

    assert!(long_area.height > short_area.height);
    assert!(endpoint_end_row > endpoint_start_row);
    assert!(footer_row > endpoint_end_row);
    assert!(footer_row < long_area.bottom() - 1);
}

#[test]
fn rendered_preview_tracks_the_active_base_url_edit_buffer() {
    let mut popup = make_popup("custom/model", "https://saved.example");
    popup.cursor = 2;
    popup.start_editing();
    popup.edit_buffer = "https://draft.example/custom#".to_string();
    popup.edit_cursor = popup.edit_buffer.chars().count();

    let (buffer, area) = render_test_popup(&popup, 120, 30);

    find_text(
        &buffer,
        area,
        "POST https://draft.example/custom",
        area.top(),
    );
}

#[test]
fn api_key_provenance_and_resolved_endpoint_share_the_status_area() {
    let mut popup = make_popup("custom/model", "https://gateway.example");
    popup.api_key_status = Some("set by HNTUI_LLM_API_KEY".to_string());
    popup.saved_at = Some(Instant::now());

    let (buffer, area) = render_test_popup(&popup, 120, 30);
    let (_, key_row) = find_text(&buffer, area, "set by HNTUI_LLM_API_KEY", area.top());
    let (_, endpoint_row) = find_text(&buffer, area, "POST https://", key_row);

    assert_eq!(endpoint_row, key_row + 1);
}

#[test]
fn connection_test_is_rendered_as_the_eighth_navigable_row() {
    let mut popup = make_popup("custom/model", "https://gateway.example");
    popup.cursor = 7;

    let (buffer, area) = render_test_popup(&popup, 120, 30);
    let (column, row) = find_text(&buffer, area, "> [ Test connection ]", area.top());

    assert_eq!(buffer[(column, row)].fg, theme::MAUVE);
}

#[test]
fn connection_test_states_use_their_semantic_styles() {
    let cases = [
        (
            crate::app::ConnectionTestState::Testing,
            "testing…".to_string(),
            theme::SUBTEXT0,
        ),
        (
            crate::app::ConnectionTestState::Success {
                model: "fallback/served-model".to_string(),
                ttft: Duration::from_millis(125),
            },
            "ok · fallback/served-model · 125ms".to_string(),
            theme::GREEN,
        ),
        (
            crate::app::ConnectionTestState::Error("check API key · invalid token".to_string()),
            "check API key · invalid token".to_string(),
            theme::RED,
        ),
    ];

    for (state, expected, color) in cases {
        let mut popup = make_popup("custom/model", "https://gateway.example");
        popup.connection_test = state;
        let (buffer, area) = render_test_popup(&popup, 120, 30);
        let (column, row) = find_text(&buffer, area, &expected, area.top());
        let first_symbol = expected
            .chars()
            .next()
            .expect("connection-test status text")
            .to_string();
        assert_eq!(
            buffer[(column, row)].symbol(),
            first_symbol,
            "coordinate must point at the first matched cell after the Unicode icon"
        );
        assert_eq!(buffer[(column, row)].fg, color, "for {expected}");
    }
}

#[test]
fn a_bounded_connection_error_wraps_to_at_most_three_lines() {
    let mut popup = make_popup("custom/model", "https://gateway.example");
    popup.connection_test =
        crate::app::ConnectionTestState::Error(format!("check API key · {}END", "x".repeat(120)));

    let (buffer, area) = render_test_popup(&popup, 70, 40);
    let (_, start_row) = find_text(&buffer, area, "✗ check API key", area.top());
    let (_, end_row) = find_text(&buffer, area, "END", start_row);

    assert!(end_row - start_row <= 2);
}
