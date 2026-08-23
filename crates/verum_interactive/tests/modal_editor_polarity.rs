//! The modal fullscreen editor, exercised through the public surface
//! (keys + render) — the notebook must vanish under the modal and come
//! back when it collapses (owner request 2026-08-23).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use verum_interactive::PlaybookApp;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let area = *buffer.area();
    let mut out = String::new();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            out.push_str(buffer[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn draw(app: &PlaybookApp) -> String {
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|f| app.render(f)).expect("draw");
    buffer_text(&terminal)
}

/// Ctrl+F in the editor expands it over the whole frame: the notebook
/// panel disappears; the first Esc collapses the modal (notebook
/// returns, still editing); the second Esc leaves edit mode.
#[test]
fn modal_fullscreen_covers_and_esc_unwinds_in_two_steps() {
    let mut app = PlaybookApp::new();
    app.session
        .update_current_source("fn main() -> Int { 42 }");
    app.handle_key(key(KeyCode::Enter)); // enter edit mode
    let normal = draw(&app);
    assert!(
        normal.contains("VERUM PLAYBOOK"),
        "notebook header visible before the modal:\n{normal}"
    );

    app.handle_key(ctrl('f'));
    let modal = draw(&app);
    assert!(
        modal.contains("FULLSCREEN"),
        "modal editor is on and labeled:\n{modal}"
    );
    assert!(
        !modal.contains("VERUM PLAYBOOK"),
        "the notebook is hidden under the modal:\n{modal}"
    );

    app.handle_key(key(KeyCode::Esc));
    let collapsed = draw(&app);
    assert!(
        collapsed.contains("VERUM PLAYBOOK"),
        "first Esc collapses the modal back to the notebook:\n{collapsed}"
    );
    assert!(
        !collapsed.contains("FULLSCREEN"),
        "modal label gone after collapse:\n{collapsed}"
    );

    // Still in edit mode: the editor status hints stay editor-flavored.
    app.handle_key(key(KeyCode::Esc));
    let left = draw(&app);
    assert!(
        left.contains("Enter:edit") || left.contains("i:edit"),
        "second Esc leaves edit mode (normal-mode hints):\n{left}"
    );
}

/// The editor title carries the live Ln/Col status — it moves when the
/// cursor moves.
#[test]
fn editor_title_reports_position() {
    let mut app = PlaybookApp::new();
    app.session
        .update_current_source("abc\ndef");
    app.handle_key(key(KeyCode::Enter));
    let before = draw(&app);
    assert!(
        before.contains("Ln 2, Col 4 (2 lines)"),
        "entering edit lands at the end of the (synced) buffer:\n{before}"
    );
    app.handle_key(key(KeyCode::Up));
    app.handle_key(key(KeyCode::Home));
    let after = draw(&app);
    assert!(
        after.contains("Ln 1, Col 1"),
        "cursor movement is reflected in the title:\n{after}"
    );
}
