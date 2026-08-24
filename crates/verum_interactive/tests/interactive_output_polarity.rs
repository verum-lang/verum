//! The interactive path must deliver what the headless path delivers.
//!
//! A cell run from the TUI goes through a DIFFERENT engine than the
//! sync path (`SessionState::execute_current`): the app prepares the
//! grown-module question, hands it to a throwaway `ScriptEngine` on a
//! worker thread, and applies the outcome back. Anything the throwaway
//! engine is configured with differently — capture, permissions,
//! entry resolution — shows up as "the cell ran fine and printed
//! nothing", which is exactly the class this file pins.
//!
//! Both polarities: the sync carrier AND the worker carrier must carry
//! stdout to the cell.

use verum_interactive::SessionState;

/// The worker-side engine of the interactive path, configured exactly
/// as `PlaybookApp::execute_current_cell` configures it.
fn worker_engine() -> verum_vbc::interpreter::ScriptEngine {
    verum_vbc::interpreter::ScriptEngine::new()
        .allow_file_io()
        .allow_network()
        .allow_process()
}

#[test]
fn sync_path_carries_stdout_to_the_cell() {
    verum_compiler::api::ensure_scripting_compiler_installed();
    let mut session = SessionState::new();
    session.update_current_source("print(\"beacon-sync\");");
    session.execute_current().expect("cell runs");
    let out = format!("{:?}", session.cells[0].output);
    assert!(
        out.contains("beacon-sync"),
        "sync path must show the printed line; got: {out}"
    );
}

#[test]
fn worker_path_carries_stdout_to_the_cell() {
    verum_compiler::api::ensure_scripting_compiler_installed();
    let mut session = SessionState::new();
    session.update_current_source("print(\"beacon-worker\");");

    // Exactly the interactive sequence: prepare here, run on a
    // throwaway engine (as the worker thread does), apply back.
    let (source, indices) = session
        .prepare_notebook_run(0)
        .expect("a code cell prepares a run");
    let mut engine = worker_engine();
    let started = std::time::Instant::now();
    let outcome = verum_interactive::playbook::session::run_grown_module(&mut engine, &source);
    let elapsed = started.elapsed();
    assert!(
        outcome.stdout.contains("beacon-worker"),
        "the worker engine must CAPTURE stdout; got stdout={:?} error={:?}",
        outcome.stdout,
        outcome.error
    );

    session
        .apply_notebook_outcome(&indices, outcome, elapsed)
        .expect("outcome applies");
    let out = format!("{:?}", session.cells[0].output);
    assert!(
        out.contains("beacon-worker"),
        "worker path must show the printed line; got: {out}"
    );
}

/// Multi-line output survives whole — a cell that prints several
/// lines shows all of them, not just the first.
#[test]
fn every_printed_line_reaches_the_cell() {
    verum_compiler::api::ensure_scripting_compiler_installed();
    let mut session = SessionState::new();
    session.update_current_source(
        "print(\"line-one\");\nprint(\"line-two\");\nprint(\"line-three\");",
    );
    session.execute_current().expect("cell runs");
    let out = format!("{:?}", session.cells[0].output);
    for needle in ["line-one", "line-two", "line-three"] {
        assert!(out.contains(needle), "missing {needle} in {out}");
    }
}
