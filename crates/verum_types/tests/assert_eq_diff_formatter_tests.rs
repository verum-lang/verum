#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! assert_eq structural diff formatter drift guard (#67).
//!
//! `vcs/runner/vtest/src/report.rs` provides `generate_diff` and
//! `generate_unified_diff` using the `similar` crate.  On assert_eq failures
//! the runner includes a structured diff in the failure output.
//!
//! This drift guard pins:
//!   1. report.rs imports/uses `similar` for diffing.
//!   2. report.rs exports `generate_diff` function.
//!   3. report.rs exports `generate_unified_diff` function.
//!   4. generate_diff accepts `expected` and `actual` str params.
//!   5. VCS spec is `@test: typecheck-pass`.
//!   6. VCS spec uses assert_eq.
//!   7. VCS spec uses assert_ne.
//!   8. VCS spec models diff-empty-on-identical contract.
//!   9. VCS spec models diff-nonempty-on-distinct contract.

const REPORT_SRC: &str = include_str!("../../../vcs/runner/vtest/src/report.rs");
const SPEC: &str = include_str!(
    "../../../vcs/specs/L2-standard/testing/assert_eq_diff_formatter.vr"
);

// ── 1. report.rs uses similar ─────────────────────────────────────────────────

#[test]
fn report_uses_similar_crate() {
    assert!(
        REPORT_SRC.contains("similar"),
        "report.rs must use the 'similar' crate for structural diffs"
    );
}

// ── 2. generate_diff exported ────────────────────────────────────────────────

#[test]
fn report_exports_generate_diff() {
    assert!(
        REPORT_SRC.contains("pub fn generate_diff"),
        "report.rs must export 'generate_diff'"
    );
}

// ── 3. generate_unified_diff exported ────────────────────────────────────────

#[test]
fn report_exports_generate_unified_diff() {
    assert!(
        REPORT_SRC.contains("pub fn generate_unified_diff"),
        "report.rs must export 'generate_unified_diff'"
    );
}

// ── 4. generate_diff parameter shape ─────────────────────────────────────────

#[test]
fn generate_diff_has_expected_and_actual_params() {
    assert!(
        REPORT_SRC.contains("expected: &str") && REPORT_SRC.contains("actual: &str"),
        "generate_diff must take 'expected: &str' and 'actual: &str' parameters"
    );
}

// ── 5. VCS spec is typecheck-pass ─────────────────────────────────────────────

#[test]
fn spec_is_typecheck_pass() {
    assert!(
        SPEC.contains("@test: typecheck-pass"),
        "assert_eq_diff_formatter.vr must be '@test: typecheck-pass'"
    );
}

// ── 6. VCS spec uses assert_eq ────────────────────────────────────────────────

#[test]
fn spec_uses_assert_eq() {
    assert!(
        SPEC.contains("assert_eq("),
        "assert_eq_diff_formatter.vr must use 'assert_eq('"
    );
}

// ── 7. VCS spec uses assert_ne ────────────────────────────────────────────────

#[test]
fn spec_uses_assert_ne() {
    assert!(
        SPEC.contains("assert_ne("),
        "assert_eq_diff_formatter.vr must use 'assert_ne('"
    );
}

// ── 8. Diff-empty-on-identical contract modelled ──────────────────────────────

#[test]
fn spec_models_diff_empty_on_identical() {
    assert!(
        SPEC.contains("diff_is_empty_for_identical") || SPEC.contains("identical"),
        "assert_eq_diff_formatter.vr must model 'diff empty on identical inputs' contract"
    );
}

// ── 9. Diff-nonempty-on-distinct contract modelled ───────────────────────────

#[test]
fn spec_models_diff_nonempty_on_distinct() {
    assert!(
        SPEC.contains("diff_would_be_nonempty_for_distinct") || SPEC.contains("distinct"),
        "assert_eq_diff_formatter.vr must model 'diff non-empty on distinct inputs' contract"
    );
}
