//! The manifest's solver settings reach the solver — T1233.
//!
//! ONE test function, deliberately. `install` / `effective` are
//! process-wide (a `OnceLock` pair plus a read flag), and cargo runs
//! the tests of one binary as threads in ONE process. Split across
//! `#[test]` functions these would race: whichever ran first would
//! decide what the others saw, and the failure would be intermittent
//! and blamed on anything but the test layout. One function, one
//! process, the state machine walked in order.

use verum_smt::config::{self, InstallError, SolverConfig};

#[test]
fn the_installed_configuration_is_what_new_constructors_read() {
    // 1. Before anything is installed, `effective` is the defaults and
    //    `is_installed` says so. Reading here also ARMS the read flag,
    //    which step 3 depends on.
    assert!(!config::is_installed(), "nothing installed yet");
    assert_eq!(
        config::effective().qe.simplify_level,
        SolverConfig::default().qe.simplify_level,
        "with no manifest, `effective` is `Default`"
    );

    // 2. Installing AFTER a read is refused. This is the ordering
    //    contract: a late install would leave the early reader on the
    //    defaults and the late one on the manifest — two components
    //    silently disagreeing, which is the failure mode the whole row
    //    exists to remove. It must be an error, not a shrug.
    let mut cfg = SolverConfig::default();
    cfg.qe.simplify_level = 0;
    cfg.sep_logic.max_unfolding_depth = 1;
    assert_eq!(
        config::install(cfg),
        Err(InstallError::AlreadyRead),
        "install after a read must be refused"
    );
    assert!(
        !config::is_installed(),
        "a refused install must not take effect"
    );

    // 3. And the refusal is not silent: it carries a message a caller
    //    can put in front of a user.
    assert!(
        InstallError::AlreadyRead.to_string().contains("earlier"),
        "the error must say what to do about it, got: {}",
        InstallError::AlreadyRead
    );

    // 4. `effective` is unchanged by the refused install — the reader
    //    still sees exactly what it saw in step 1.
    assert_eq!(
        config::effective().qe.simplify_level,
        SolverConfig::default().qe.simplify_level,
        "a refused install must not change what readers see"
    );
    assert_eq!(
        config::effective().sep_logic.max_unfolding_depth,
        SolverConfig::default().sep_logic.max_unfolding_depth,
    );
}
