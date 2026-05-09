#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! @fuzz attribute / fuzzing harness directive drift guard (#68).
//!
//! `vcs/runner/vtest/src/fuzz.rs` provides the fuzzing infrastructure.
//! `directive.rs` has a `fuzz_entry` field parsed from `@fuzz: <fn_name>`.
//!
//! This drift guard pins:
//!   1. directive.rs has a `fuzz_entry` field on TestDirectives.
//!   2. `@fuzz:` prefix is parsed into fuzz_entry.
//!   3. fuzz_entry defaults to None.
//!   4. fuzz.rs defines FuzzConfig.
//!   5. fuzz.rs has a `corpus_dir` field in FuzzConfig.
//!   6. fuzz.rs has a `crashes_dir` field in FuzzConfig.
//!   7. VCS spec is `@test: typecheck-pass`.
//!   8. VCS spec uses `@fuzz:` directive.
//!   9. VCS spec defines a fuzz entry function accepting &[Byte].

const DIRECTIVE_SRC: &str = include_str!("../../../vcs/runner/vtest/src/directive.rs");
const FUZZ_SRC: &str = include_str!("../../../vcs/runner/vtest/src/fuzz.rs");
const SPEC: &str = include_str!(
    "../../../vcs/specs/L2-standard/testing/fuzz_harness_attribute.vr"
);

// ── 1. fuzz_entry field on TestDirectives ────────────────────────────────────

#[test]
fn directive_has_fuzz_entry_field() {
    assert!(
        DIRECTIVE_SRC.contains("fuzz_entry"),
        "TestDirectives must have a 'fuzz_entry' field"
    );
}

// ── 2. @fuzz: prefix parsed ───────────────────────────────────────────────────

#[test]
fn directive_parses_fuzz_prefix() {
    assert!(
        DIRECTIVE_SRC.contains("@fuzz:"),
        "directive.rs must parse '@fuzz:' prefix"
    );
}

// ── 3. fuzz_entry defaults to None ───────────────────────────────────────────

#[test]
fn fuzz_entry_defaults_to_none() {
    assert!(
        DIRECTIVE_SRC.contains("fuzz_entry: None"),
        "directive.rs default must set fuzz_entry to None"
    );
}

// ── 4. fuzz.rs defines FuzzConfig ────────────────────────────────────────────

#[test]
fn fuzz_rs_defines_fuzz_config() {
    assert!(
        FUZZ_SRC.contains("FuzzConfig"),
        "fuzz.rs must define 'FuzzConfig'"
    );
}

// ── 5. FuzzConfig has corpus_dir ─────────────────────────────────────────────

#[test]
fn fuzz_config_has_corpus_dir() {
    assert!(
        FUZZ_SRC.contains("corpus_dir"),
        "FuzzConfig must have a 'corpus_dir' field"
    );
}

// ── 6. FuzzConfig has crashes_dir ────────────────────────────────────────────

#[test]
fn fuzz_config_has_crashes_dir() {
    assert!(
        FUZZ_SRC.contains("crashes_dir"),
        "FuzzConfig must have a 'crashes_dir' field"
    );
}

// ── 7. VCS spec is typecheck-pass ─────────────────────────────────────────────

#[test]
fn spec_is_typecheck_pass() {
    assert!(
        SPEC.contains("@test: typecheck-pass"),
        "fuzz_harness_attribute.vr must be '@test: typecheck-pass'"
    );
}

// ── 8. VCS spec uses @fuzz directive ─────────────────────────────────────────

#[test]
fn spec_uses_fuzz_directive() {
    assert!(
        SPEC.contains("@fuzz:"),
        "fuzz_harness_attribute.vr must use '@fuzz:' directive"
    );
}

// ── 9. VCS spec defines a fuzz entry function ────────────────────────────────

#[test]
fn spec_defines_fuzz_entry_function() {
    assert!(
        SPEC.contains("fn fuzz_"),
        "fuzz_harness_attribute.vr must define a 'fn fuzz_*' entry function"
    );
}
