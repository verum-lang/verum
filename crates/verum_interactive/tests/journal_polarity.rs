//! The Journal lens (T0858 slice 5): every question the session asks
//! lands in the ledger — and a session that asked nothing shows an
//! honest empty state, not an invented history.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use verum_interactive::PlaybookApp;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn draw(app: &PlaybookApp) -> String {
    let backend = TestBackend::new(140, 40);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|f| app.render(f)).expect("draw");
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

/// Tab to the Journal lens (5 tabs from Variables).
fn goto_journal(app: &mut PlaybookApp) {
    for _ in 0..5 {
        app.handle_key(key(KeyCode::Tab));
    }
}

#[test]
fn a_quiet_session_shows_an_honest_empty_ledger() {
    let mut app = PlaybookApp::new();
    goto_journal(&mut app);
    let text = draw(&app);
    assert!(text.contains(" Journal "), "Journal tab active:\n{text}");
    assert!(
        text.contains("nothing asked yet"),
        "empty state, not invented history:\n{text}"
    );
}

#[test]
fn lens_queries_land_in_the_ledger() {
    let mut app = PlaybookApp::new();
    app.session.update_current_source("fn main() -> Int { 0 }");
    // Landing on Arch runs arch.query (journalled), then walk on to
    // the Journal tab.
    goto_journal(&mut app);
    let text = draw(&app);
    assert!(
        text.contains("arch.query"),
        "the arch query the walk-through triggered is in the ledger:\n{text}"
    );
}
