//! T0671 leg 3 — the LIVE `verum verify` path honours the
//! Thorough/Certified strategy rungs.
//!
//! Increments 1-2 wired mandatory termination through the
//! `verum_verification` stack (VCGenerator / SmtVerificationPass /
//! verify-ladder). This pins the remaining leg: the pipeline that
//! `verum verify` actually runs (`verify_cmd::VerifyCommand`)
//! enforces the same contract once `CompilerOptions.verify_strategy`
//! says Thorough — a while-loop with NO `decreases` measure FAILS,
//! the same loop with a valid measure PASSES, and (the wiring proof)
//! the identical missing-measure file still passes at the default
//! Proof rung. A strategy flag that changes nothing is
//! indistinguishable from an unwired one.
//!
//! NOTE: `crates/*/tests/` does not gate in CI until T0709 lands;
//! placed here per repository layout policy regardless.

use std::io::Write;

use verum_compiler::verify_cmd::VerifyCommand;
use verum_compiler::{CompilerOptions, Session, VerifyMode, VerifyStrategy};

const MISSING_MEASURE: &str = r#"
fn count_up(n: Int) -> Int {
    let mut i = 0;
    let mut acc = 0;
    while i < n {
        acc = acc + i;
        i = i + 1;
    }
    acc
}

fn main() { count_up(3); }
"#;

const VALID_MEASURE: &str = r#"
fn count_up(n: Int) -> Int {
    let mut i = 0;
    let mut acc = 0;
    while i < n
        invariant i >= 0
        decreases n - i
    {
        acc = acc + i;
        i = i + 1;
    }
    acc
}

fn main() { count_up(3); }
"#;

fn run_verify(source: &str, strategy: VerifyStrategy) -> Result<(), String> {
    let mut file = tempfile::Builder::new()
        .suffix(".vr")
        .tempfile()
        .expect("tempfile");
    file.write_all(source.as_bytes()).expect("write source");
    let path = file.path().to_path_buf();

    let options = CompilerOptions {
        input: path,
        verify_mode: VerifyMode::Proof,
        verify_strategy: strategy,
        smt_timeout_secs: 30,
        ..Default::default()
    };
    let mut session = Session::new(options);
    let cmd = VerifyCommand::new(&mut session);
    cmd.run(None).map_err(|e| e.to_string())
}

#[test]
fn thorough_fails_missing_measure() {
    let err = run_verify(MISSING_MEASURE, VerifyStrategy::Thorough)
        .expect_err("a measureless loop MUST fail --mode=thorough");
    assert!(
        err.contains("Verification failed"),
        "failure must be a verification verdict, got: {err}"
    );
}

#[test]
fn thorough_passes_valid_measure() {
    run_verify(VALID_MEASURE, VerifyStrategy::Thorough)
        .expect("a well-founded decreases measure must pass --mode=thorough");
}

#[test]
fn proof_rung_stays_lenient_on_missing_measure() {
    // The wiring proof: the SAME file that Thorough rejects must
    // still pass at the default rung — otherwise the strategy flag
    // changed nothing and the two tests above prove nothing.
    run_verify(MISSING_MEASURE, VerifyStrategy::Proof)
        .expect("the default Proof rung must not demand measures");
}

#[test]
fn certified_fails_missing_measure_too() {
    let err = run_verify(MISSING_MEASURE, VerifyStrategy::Certified)
        .expect_err("Certified includes Thorough's mandatoriness");
    assert!(err.contains("Verification failed"), "got: {err}");
}
