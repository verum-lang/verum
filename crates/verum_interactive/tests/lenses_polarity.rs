//! Both-polarity checks for the VBC and Tiers lenses (T0858 slice 3),
//! driven through the PUBLIC surface only (keys + render), the same
//! way a hand on the keyboard reaches them.

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

fn draw(app: &PlaybookApp) -> String {
    let backend = TestBackend::new(140, 40);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|f| app.render(f)).expect("draw");
    buffer_text(&terminal)
}

/// Tab into the VBC lens: the disassembly of the notebook-as-module
/// is on screen — from the same artifact path the interpreter runs.
#[test]
fn vbc_lens_shows_the_notebook_bytecode() {
    verum_compiler::api::ensure_scripting_compiler_installed();
    let mut app = PlaybookApp::new();
    app.session
        .update_current_source("fn main() -> Int { 40 + 2 }");
    // Variables → Outline → Arch → VBC.
    app.handle_key(key(KeyCode::Tab));
    app.handle_key(key(KeyCode::Tab));
    app.handle_key(key(KeyCode::Tab));
    let text = draw(&app);
    assert!(text.contains(" VBC "), "VBC tab is active:\n{text}");
    assert!(
        text.contains("VBC Module"),
        "the disassembly header is on screen:\n{text}"
    );
}

/// The Tiers lens never judges uninvited: entering the tab shows the
/// price-tagged hint, not a verdict.
#[test]
fn tiers_lens_waits_for_an_explicit_ask() {
    let mut app = PlaybookApp::new();
    app.session
        .update_current_source("fn main() -> Int { 0 }");
    for _ in 0..4 {
        app.handle_key(key(KeyCode::Tab));
    }
    let text = draw(&app);
    assert!(text.contains(" Tiers "), "Tiers tab is active:\n{text}");
    assert!(
        text.contains("press t to judge"),
        "the on-demand hint is shown, no judgment ran:\n{text}"
    );
}

/// An empty notebook refuses the judgment honestly instead of
/// spawning subprocesses over nothing.
#[test]
fn tiers_judge_refuses_an_empty_notebook() {
    let mut app = PlaybookApp::new();
    for _ in 0..4 {
        app.handle_key(key(KeyCode::Tab));
    }
    app.handle_key(key(KeyCode::Char('t')));
    let text = draw(&app);
    assert!(
        text.contains("empty notebook"),
        "the refusal names the reason:\n{text}"
    );
}

/// Negative control: with the sidebar on any OTHER tab, `t` does not
/// start a judgment — the trigger is scoped to the lens it serves.
#[test]
fn t_key_outside_the_tiers_lens_is_inert() {
    let mut app = PlaybookApp::new();
    app.session
        .update_current_source("fn main() -> Int { 0 }");
    app.handle_key(key(KeyCode::Char('t')));
    let text = draw(&app);
    assert!(
        !text.contains("judging"),
        "no judgment starts from the Variables tab:\n{text}"
    );
}
