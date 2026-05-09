#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! JSON event schema versioning drift guard (#70).
//!
//! `vcs/runner/vtest/src/report.rs` exports the `Report` struct serialised as
//! JSON when `--format json` is passed.  The struct includes a `schema_version`
//! field (currently 1) so consumers can reject reports from incompatible runner
//! versions.
//!
//! This drift guard pins:
//!   1. report.rs defines REPORT_SCHEMA_VERSION constant.
//!   2. REPORT_SCHEMA_VERSION equals 1.
//!   3. Report struct has a `schema_version` field.
//!   4. schema_version has a serde default annotation.
//!   5. report.rs has a `default_schema_version` function.
//!   6. VCS spec is `@test: typecheck-pass`.
//!   7. VCS spec defines REPORT_SCHEMA_VERSION constant.
//!   8. VCS spec tests that REPORT_SCHEMA_VERSION equals 1.
//!   9. VCS spec tests version-supported / unsupported logic.

const REPORT_SRC: &str = include_str!("../../../vcs/runner/vtest/src/report.rs");
const SPEC: &str = include_str!(
    "../../../vcs/specs/L2-standard/testing/json_event_schema_version.vr"
);

// ── 1. REPORT_SCHEMA_VERSION constant defined ─────────────────────────────────

#[test]
fn report_defines_schema_version_constant() {
    assert!(
        REPORT_SRC.contains("REPORT_SCHEMA_VERSION"),
        "report.rs must define 'REPORT_SCHEMA_VERSION' constant"
    );
}

// ── 2. Constant equals 1 ─────────────────────────────────────────────────────

#[test]
fn schema_version_constant_is_one() {
    assert!(
        REPORT_SRC.contains("REPORT_SCHEMA_VERSION: u32 = 1"),
        "REPORT_SCHEMA_VERSION must be 1"
    );
}

// ── 3. Report struct has schema_version field ─────────────────────────────────

#[test]
fn report_struct_has_schema_version_field() {
    assert!(
        REPORT_SRC.contains("schema_version"),
        "Report struct must have a 'schema_version' field"
    );
}

// ── 4. schema_version has serde default annotation ───────────────────────────

#[test]
fn schema_version_has_serde_default() {
    assert!(
        REPORT_SRC.contains("#[serde(default"),
        "schema_version field must carry a '#[serde(default…)]' annotation"
    );
}

// ── 5. default_schema_version function ───────────────────────────────────────

#[test]
fn report_has_default_schema_version_fn() {
    assert!(
        REPORT_SRC.contains("fn default_schema_version"),
        "report.rs must have a 'default_schema_version' function"
    );
}

// ── 6. VCS spec is typecheck-pass ─────────────────────────────────────────────

#[test]
fn spec_is_typecheck_pass() {
    assert!(
        SPEC.contains("@test: typecheck-pass"),
        "json_event_schema_version.vr must be '@test: typecheck-pass'"
    );
}

// ── 7. VCS spec defines REPORT_SCHEMA_VERSION ────────────────────────────────

#[test]
fn spec_defines_schema_version_constant() {
    assert!(
        SPEC.contains("REPORT_SCHEMA_VERSION"),
        "json_event_schema_version.vr must define 'REPORT_SCHEMA_VERSION'"
    );
}

// ── 8. VCS spec asserts version equals 1 ─────────────────────────────────────

#[test]
fn spec_asserts_version_is_one() {
    assert!(
        SPEC.contains("assert_eq(REPORT_SCHEMA_VERSION, 1)")
            || SPEC.contains("REPORT_SCHEMA_VERSION, 1"),
        "json_event_schema_version.vr must assert REPORT_SCHEMA_VERSION == 1"
    );
}

// ── 9. VCS spec tests version support logic ───────────────────────────────────

#[test]
fn spec_tests_version_supported_logic() {
    assert!(
        SPEC.contains("is_version_supported") || SPEC.contains("version_supported"),
        "json_event_schema_version.vr must test version-supported logic"
    );
}
