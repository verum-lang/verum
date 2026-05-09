#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! Glob negated character class [!abc] drift guard (#75).
//!
//! `core/base/glob.vr` implements glob pattern matching with support for
//! negated character classes `[!abc]` (also accepting `^` as the negation
//! indicator).
//!
//! This drift guard pins:
//!   1. glob.vr defines `GlobPart::Class` with a `negated: Bool` field.
//!   2. glob.vr parses `!` and `^` as the class negation indicator.
//!   3. glob.vr inverts the match when `negated` is true.
//!   4. GlobError has UnclosedClass / EmptyClass / TrailingEscape variants.
//!   5. glob.vr exports `pub fn compile`.
//!   6. glob.vr exports `pub fn matches`.
//!   7. GlobPattern has a `parts` field.
//!   8. VCS spec uses `compile`, `matches`, `GlobError`.

const GLOB_VR: &str = include_str!("../../../core/base/glob.vr");
const GLOB_SPEC: &str = include_str!(
    "../../../vcs/specs/L2-standard/stdlib/glob_negated_class.vr"
);

// ── 1. GlobPart::Class has negated: Bool ─────────────────────────────────────

#[test]
fn glob_part_class_has_negated_field() {
    assert!(
        GLOB_VR.contains("negated: Bool"),
        "GlobPart::Class must have 'negated: Bool' field"
    );
}

// ── 2. Parser recognises ! and ^ as negation ────────────────────────────────

#[test]
fn glob_vr_parses_exclamation_negation() {
    assert!(
        GLOB_VR.contains("negated = true"),
        "glob.vr must set 'negated = true' when '!' is found"
    );
}

// ── 3. Match inverts when negated ─────────────────────────────────────────────

#[test]
fn glob_vr_inverts_match_for_negated_class() {
    assert!(
        GLOB_VR.contains("*negated") || GLOB_VR.contains("negated &&"),
        "glob.vr matches_impl must check 'negated' to invert the class match"
    );
}

// ── 4. GlobError variants ─────────────────────────────────────────────────────

#[test]
fn glob_error_has_unclosed_class() {
    assert!(
        GLOB_VR.contains("UnclosedClass"),
        "GlobError must have UnclosedClass variant"
    );
}

#[test]
fn glob_error_has_empty_class() {
    assert!(
        GLOB_VR.contains("EmptyClass"),
        "GlobError must have EmptyClass variant"
    );
}

#[test]
fn glob_error_has_trailing_escape() {
    assert!(
        GLOB_VR.contains("TrailingEscape"),
        "GlobError must have TrailingEscape variant"
    );
}

// ── 5. pub fn compile ─────────────────────────────────────────────────────────

#[test]
fn glob_vr_has_pub_fn_compile() {
    assert!(
        GLOB_VR.contains("pub fn compile") || GLOB_VR.contains("public fn compile"),
        "glob.vr must export a 'compile' function"
    );
}

// ── 6. pub fn matches ─────────────────────────────────────────────────────────

#[test]
fn glob_vr_has_pub_fn_matches() {
    assert!(
        GLOB_VR.contains("pub fn matches") || GLOB_VR.contains("public fn matches"),
        "glob.vr must export a 'matches' function"
    );
}

// ── 7. GlobPattern has parts field ───────────────────────────────────────────

#[test]
fn glob_pattern_has_parts_field() {
    assert!(
        GLOB_VR.contains("parts:"),
        "GlobPattern must have a 'parts' field"
    );
}

// ── 8. VCS spec ───────────────────────────────────────────────────────────────

#[test]
fn glob_spec_uses_compile() {
    assert!(
        GLOB_SPEC.contains("compile("),
        "glob_negated_class.vr must call 'compile('"
    );
}

#[test]
fn glob_spec_uses_matches() {
    assert!(
        GLOB_SPEC.contains("matches("),
        "glob_negated_class.vr must call 'matches('"
    );
}

#[test]
fn glob_spec_uses_glob_error() {
    assert!(
        GLOB_SPEC.contains("GlobError"),
        "glob_negated_class.vr must reference GlobError"
    );
}

#[test]
fn glob_spec_uses_negated_class_syntax() {
    assert!(
        GLOB_SPEC.contains("[!"),
        "glob_negated_class.vr must use '[!' negated class syntax"
    );
}

#[test]
fn glob_spec_is_typecheck_pass() {
    assert!(
        GLOB_SPEC.contains("@test: typecheck-pass"),
        "glob_negated_class.vr must be '@test: typecheck-pass'"
    );
}
