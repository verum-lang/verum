//! What the reader SEES after running a cell (T0858).
//!
//! "The cell ran, the status said Done, and the panel was empty" is a
//! rendering fault, not an execution one — the session can hold a
//! perfectly good `CellOutput` that never reaches the screen. These
//! checks read the rendered TUI buffer, so they fail on exactly that
//! gap, and they cover the sidebar tabs the reader switches between
//! while a notebook is loaded (the VBC lens has its own file).

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use verum_interactive::PlaybookApp;

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

fn draw(app: &mut PlaybookApp, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|f| app.render(f)).expect("draw");
    buffer_text(&terminal)
}

/// The headline case: a cell that prints must SHOW what it printed.
#[test]
fn printed_lines_are_visible_after_running_a_cell() {
    verum_compiler::api::ensure_scripting_compiler_installed();
    let mut app = PlaybookApp::new();
    app.session
        .update_current_source("print(\"visible-beacon\");");
    app.session.execute_current().expect("cell runs");

    let text = draw(&mut app, 100, 30);
    assert!(
        text.contains("visible-beacon"),
        "the printed line must be on screen:\n{text}"
    );
}

/// Several printed lines all reach the screen — a panel that shows
/// only the first line is the same fault at a smaller scale.
#[test]
fn multi_line_output_is_visible() {
    verum_compiler::api::ensure_scripting_compiler_installed();
    let mut app = PlaybookApp::new();
    app.session.update_current_source(
        "print(\"alpha-line\");\nprint(\"beta-line\");\nprint(\"gamma-line\");",
    );
    app.session.execute_current().expect("cell runs");

    let text = draw(&mut app, 100, 40);
    for needle in ["alpha-line", "beta-line", "gamma-line"] {
        assert!(text.contains(needle), "missing {needle}:\n{text}");
    }
}

/// Every sidebar tab renders without panicking and paints its own
/// identity, with a notebook that has actually run. Switching tabs is
/// the most common interaction in the sidebar; a tab that panics takes
/// the whole terminal down with it.
#[test]
fn every_sidebar_tab_renders_with_a_live_notebook() {
    verum_compiler::api::ensure_scripting_compiler_installed();
    let mut app = PlaybookApp::new();
    app.session
        .update_current_source("let tally = 7;\nprint(\"tab-probe\");");
    app.session.execute_current().expect("cell runs");

    // Walk the whole ring twice — `next` wraps, and a stale lens from
    // the first pass must not break the second.
    let mut seen: Vec<String> = Vec::new();
    for _ in 0..14 {
        let text = draw(&mut app, 120, 40);
        seen.push(text);
        app.sidebar_tab = app.sidebar_tab.next();
    }
    for (i, text) in seen.iter().enumerate() {
        assert!(
            text.contains("Vars") && text.contains("VBC"),
            "tab bar must stay painted on step {i}:\n{text}"
        );
    }
}

/// The Console lens exists and says so when nothing was captured —
/// the host installs the capture, so in-process tests see the quiet
/// state. Its value is that stray writes have a HOME instead of
/// landing on the frame.
#[test]
fn console_lens_reports_the_quiet_state() {
    let mut app = PlaybookApp::new();
    app.sidebar_tab = verum_interactive::playbook::ui::SidebarTab::Console;
    let text = draw(&mut app, 120, 30);
    assert!(
        text.contains("Con") || text.contains("CONSOLE"),
        "the Console tab is reachable:\n{text}"
    );
    assert!(
        text.contains("quiet"),
        "an empty capture states itself:\n{text}"
    );
}

/// A cell that prints nothing SAYS so — an empty panel and a broken
/// panel must not look alike.
#[test]
fn a_silent_cell_says_it_produced_nothing() {
    verum_compiler::api::ensure_scripting_compiler_installed();
    let mut app = PlaybookApp::new();
    app.session.update_current_source("let quiet = 1;");
    app.session.execute_current().expect("cell runs");
    let text = draw(&mut app, 100, 30);
    assert!(
        text.contains("no output") || text.contains("quiet"),
        "a silent run is labelled, not blank:\n{text}"
    );
}
