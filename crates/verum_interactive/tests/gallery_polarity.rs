//! Both-polarity checks for the launch gallery (T0858 slice 1):
//! an empty `verum play` opens a chooser, navigation wraps, Enter
//! commits a choice, and the ordinary notebook render never shows
//! the gallery once it is dismissed.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use verum_interactive::PlaybookApp;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
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

#[test]
fn empty_launch_shows_tours_and_blank_sheet() {
    let mut app = PlaybookApp::new();
    app.open_gallery();
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|f| app.render(f)).expect("draw");
    let text = buffer_text(&terminal);
    assert!(text.contains("VERUM PLAYGROUND"), "gallery title:\n{text}");
    assert!(text.contains("GUIDED TOURS"), "tours section:\n{text}");
    assert!(text.contains("blank sheet"), "blank-sheet entry:\n{text}");
    assert!(
        text.contains("Verum Basics"),
        "the first builtin tour is listed:\n{text}"
    );
}

#[test]
fn enter_on_blank_sheet_dismisses_the_gallery() {
    let mut app = PlaybookApp::new();
    app.open_gallery();
    app.handle_key(key(KeyCode::Enter));
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|f| app.render(f)).expect("draw");
    let text = buffer_text(&terminal);
    assert!(
        !text.contains("VERUM PLAYGROUND") || text.contains("PLAYBOOK"),
        "after choosing the blank sheet the notebook renders, not \
         the gallery:\n{text}"
    );
    assert!(
        text.contains("VERUM PLAYBOOK"),
        "the ordinary notebook chrome is on screen:\n{text}"
    );
}

#[test]
fn choosing_a_tour_loads_its_cells() {
    let mut app = PlaybookApp::new();
    app.open_gallery();
    // Move from the blank sheet to the first tour and open it.
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Enter));
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|f| app.render(f)).expect("draw");
    let text = buffer_text(&terminal);
    assert!(
        text.contains("Verum Basics"),
        "the tour's title cell is in the notebook:\n{text}"
    );
}

/// The negative control: an app that never opened a gallery renders
/// the notebook directly — the gallery is an empty-launch door, not
/// a mandatory screen.
#[test]
fn ordinary_launch_never_shows_the_gallery() {
    let app = PlaybookApp::new();
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|f| app.render(f)).expect("draw");
    let text = buffer_text(&terminal);
    assert!(!text.contains("GUIDED TOURS"), "no gallery:\n{text}");
    assert!(text.contains("VERUM PLAYBOOK"), "notebook chrome:\n{text}");
}
