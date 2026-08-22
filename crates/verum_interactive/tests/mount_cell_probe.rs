//! The stdlib-reach law (T0858 slice 5): a notebook cell mounts and
//! uses the stdlib exactly like a script — the vocabulary executor
//! compiles cells through the SAME hook-installed compiler `verum
//! run` uses. (The old parallel bridge compiled cells WITHOUT the
//! stdlib link and swallowed the mount SILENTLY: result Ok, output
//! Timing{0,0}, no exp, no error.)
use verum_interactive::SessionState;

#[test]
fn cell_with_mount_reaches_stdlib() {
    verum_compiler::api::ensure_scripting_compiler_installed();
    let mut session = SessionState::new();
    session.update_current_source(
        "mount core.math.elementary.exp;\nlet e = exp(1.0);\nprint(f\"{e}\");",
    );
    let result = session.execute_current();
    let out = format!(
        "result={:?} output={:?}",
        result.is_ok(),
        session.cells[0].output
    );
    println!("{out}");
    assert!(
        out.contains("2.718"),
        "a cell must reach the stdlib through mount; got: {out}"
    );
}

/// The Vars lens shows top-level bindings with values FROM THE RUN
/// (the VARS channel), and the value-capture machinery cell stays
/// hidden.
#[test]
fn vars_channel_feeds_the_lens() {
    verum_compiler::api::ensure_scripting_compiler_installed();
    let mut session = SessionState::new();
    session.update_current_source("let tally = 5 + 2;");
    session.execute_current().expect("cell runs");
    let names: Vec<&str> = session
        .var_previews
        .iter()
        .map(|(n, _)| n.as_str())
        .collect();
    assert!(names.contains(&"tally"), "binding listed: {names:?}");
    let tally = session
        .var_previews
        .iter()
        .find(|(n, _)| n == "tally")
        .expect("tally present");
    assert_eq!(tally.1, "7", "the value comes from the actual run");
}

/// A cell ending in a bare expression reports that expression's value
/// as the cell's Value output — through the same single run.
#[test]
fn tail_expression_value_is_the_cell_output() {
    verum_compiler::api::ensure_scripting_compiler_installed();
    let mut session = SessionState::new();
    session.update_current_source("let base = 40;
base + 2");
    session.execute_current().expect("cell runs");
    let out = format!("{:?}", session.cells[0].output);
    assert!(
        out.contains("\"42\""),
        "the tail expression's value is the cell output: {out}"
    );
}
