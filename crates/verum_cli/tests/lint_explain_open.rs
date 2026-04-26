//! `verum lint --explain RULE --open` contract tests.
//!
//! The actual browser launch is non-deterministic in CI, so we
//! gate on `VERUM_OPEN_DRY_RUN=1` which prints the URL instead of
//! dispatching the platform `open`/`xdg-open`/`start` command.

use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

fn run(args: &[&str], dry_run: bool) -> std::process::Output {
    let mut cmd = Command::new(binary());
    cmd.args(args);
    if dry_run {
        cmd.env("VERUM_OPEN_DRY_RUN", "1");
    }
    cmd.output().expect("verum spawn")
}

#[test]
fn explain_open_known_rule_prints_url_in_dry_run() {
    let out = run(
        &["lint", "--explain", "redundant-refinement", "--open"],
        true,
    );
    assert!(
        out.status.success(),
        "explain --open should succeed for a known rule. stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("https://verum-lang.dev/docs/reference/lint-rules#redundant-refinement"),
        "expected URL in dry-run stdout, got: {stdout}"
    );
}

#[test]
fn explain_open_unknown_rule_errors() {
    let out = run(
        &["lint", "--explain", "totally-not-a-rule", "--open"],
        true,
    );
    assert!(
        !out.status.success(),
        "explain --open should fail on unknown rule"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("totally-not-a-rule") || stderr.contains("unknown"),
        "error should name the unknown rule, got: {stderr}"
    );
}

#[test]
fn open_without_explain_is_rejected_by_clap() {
    // clap's `requires = "explain"` should reject this combination.
    let out = run(&["lint", "--open"], false);
    assert!(
        !out.status.success(),
        "--open without --explain must be rejected"
    );
}

#[test]
fn explain_without_open_still_prints_text() {
    // Sanity: the existing text-explain path still works after
    // adding --open.
    let out = run(
        &["lint", "--explain", "redundant-refinement"],
        false,
    );
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("redundant-refinement"),
        "text explain should still mention the rule, got: {stdout}"
    );
}
