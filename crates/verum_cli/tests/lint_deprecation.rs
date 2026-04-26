//! Rule deprecation framework — verifies the public surface is
//! present and the lookup helpers behave correctly.
//!
//! The framework today ships with an EMPTY DEPRECATED_RULES list
//! — no rules are actually deprecated. These tests assert the
//! plumbing works (lookup returns Active for every existing rule,
//! the formatter handles the empty case) so the first real
//! deprecation can land with confidence.

use verum_cli::commands::lint::{
    deprecation_notice, is_deprecated, rule_status, LintRuleStatus,
};

#[test]
fn every_existing_rule_is_active_by_default() {
    for rule in &[
        "deprecated-syntax",
        "todo-in-code",
        "redundant-refinement",
        "circular-import",
        "unused-private",
    ] {
        assert!(
            !is_deprecated(rule),
            "no rule should be marked deprecated yet — `{rule}` is"
        );
        assert_eq!(
            rule_status(rule),
            LintRuleStatus::Active,
            "default status must be Active for `{rule}`"
        );
        assert!(
            deprecation_notice(rule).is_none(),
            "no notice for an Active rule"
        );
    }
}

#[test]
fn unknown_rule_is_active() {
    // The framework treats lookup misses as Active rather than
    // erroring. Validation surfaces typos elsewhere; deprecation
    // status is a separate concern.
    assert_eq!(rule_status("totally-unknown"), LintRuleStatus::Active);
}

#[test]
fn list_rules_runs_without_panic_when_zero_deprecated() {
    let bin = env!("CARGO_BIN_EXE_verum");
    let out = std::process::Command::new(bin)
        .args(["lint", "--list-rules"])
        .output()
        .expect("spawn");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // No "(N deprecated)" suffix when DEPRECATED_RULES is empty.
    assert!(
        !stdout.contains("deprecated)"),
        "unexpected deprecated marker on a clean tree, got:\n{stdout}"
    );
}
