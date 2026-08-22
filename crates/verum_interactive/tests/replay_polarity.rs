//! Both-polarity checks for the .vrbook v2 replay law (T0858 slice 4):
//! a saved book replays identically; a tampered SOURCE is caught by
//! the chain BEFORE execution; a tampered OUTPUT is caught by the
//! bit-for-bit comparison and names its cell; a v1 book (no chain)
//! still replays.

use verum_interactive::playbook::persistence::{
    ReplayVerdict, chain_of_cells, replay_book, save_playbook,
    load_playbook_file,
};
use verum_interactive::{CellKind, CellOutput, SessionState};

/// A two-cell session, executed, saved, reloaded — the round trip a
/// real book takes.
fn saved_book(dir: &tempfile::TempDir) -> verum_interactive::playbook::persistence::PlaybookFile {
    let mut session = SessionState::new();
    session.update_current_source("let answer = 41");
    session.insert_cell_after(CellKind::Code);
    session.update_current_source("answer + 1");
    session.execute_all().expect("cells run");
    let path = dir.path().join("probe.vrbook");
    save_playbook(&path, &session.cells, None).expect("save");
    load_playbook_file(&path).expect("load")
}

#[test]
fn a_saved_book_replays_identically() {
    let dir = tempfile::tempdir().expect("tempdir");
    let book = saved_book(&dir);
    assert!(book.chain.is_some(), "v2 books carry their chain");
    let (verdict, _) = replay_book(&book);
    match verdict {
        ReplayVerdict::Identical {
            compared, cells, ..
        } => {
            assert_eq!(cells, 2);
            assert!(compared >= 1, "recorded outputs were compared");
        }
        other => panic!("expected Identical, got {other:?}"),
    }
}

#[test]
fn a_tampered_source_is_caught_by_the_chain_before_execution() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut book = saved_book(&dir);
    book.cells[0].source = "let answer = 40".into();
    let (verdict, session) = replay_book(&book);
    match verdict {
        ReplayVerdict::ChainOutOfStep { cell, .. } => assert_eq!(cell, 0),
        other => panic!("expected ChainOutOfStep, got {other:?}"),
    }
    assert!(
        session.is_none(),
        "the chain law fires BEFORE any execution"
    );
}

#[test]
fn a_tampered_output_is_caught_and_names_its_cell() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut book = saved_book(&dir);
    let victim = book
        .cells
        .iter()
        .position(|c| c.output.is_some())
        .expect("an executed cell exists");
    book.cells[victim].output = Some(CellOutput::Value {
        repr: "999".into(),
        type_info: "Int".into(),
        raw: None,
    });
    let (verdict, _) = replay_book(&book);
    match verdict {
        ReplayVerdict::Divergent { cell, .. } => assert_eq!(cell, victim),
        other => panic!("expected Divergent, got {other:?}"),
    }
}

/// A v1 book (no chain recorded) still replays — the chain is derived
/// data, never a gate on old books.
#[test]
fn a_chainless_book_still_replays() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut book = saved_book(&dir);
    book.chain = None;
    let (verdict, _) = replay_book(&book);
    assert!(
        matches!(verdict, ReplayVerdict::Identical { .. }),
        "got {verdict:?}"
    );
}

/// The chain advances on code cells only; markdown rides the current
/// address — the address names the MODULE, and prose is not module.
#[test]
fn markdown_rides_the_chain_without_advancing_it() {
    let mut session = SessionState::new();
    session.update_current_source("let x = 1");
    session.insert_cell_after(CellKind::Markdown);
    session.update_current_source("# a heading");
    let chain = chain_of_cells(&session.cells);
    assert_eq!(chain[0], chain[1], "markdown does not advance the address");
}
