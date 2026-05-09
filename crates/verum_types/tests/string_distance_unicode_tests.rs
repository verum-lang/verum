#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! string_distance Unicode handling contract drift guard (#77).
//!
//! `core/base/string_distance.vr` implements Levenshtein + Jaro-Winkler.
//! The Unicode contract: all functions operate on byte slices (UTF-8 bytes);
//! a multi-byte codepoint counts as multiple bytes in distance calculations.
//!
//! This drift guard pins:
//!   1. string_distance.vr documents that it operates on byte slices.
//!   2. string_distance.vr documents the grapheme-aware limitation.
//!   3. string_distance.vr exports `pub fn levenshtein`.
//!   4. string_distance.vr exports `pub fn levenshtein_bounded`.
//!   5. string_distance.vr exports `pub fn jaro`.
//!   6. string_distance.vr exports `pub fn jaro_winkler`.
//!   7. string_distance.vr exports `pub fn closest`.
//!   8. levenshtein signature takes `&[Byte]` slices.
//!   9. VCS spec uses all five API functions.

const STRDIST_VR: &str = include_str!("../../../core/base/string_distance.vr");
const STRDIST_SPEC: &str = include_str!(
    "../../../vcs/specs/L2-standard/stdlib/string_distance_unicode.vr"
);

// ── 1. Documented as byte-slice based ────────────────────────────────────────

#[test]
fn string_distance_vr_documents_byte_slice_basis() {
    assert!(
        STRDIST_VR.contains("byte slices") || STRDIST_VR.contains("Byte"),
        "string_distance.vr must document that it operates on byte slices"
    );
}

// ── 2. Documents grapheme-aware limitation ────────────────────────────────────

#[test]
fn string_distance_vr_documents_grapheme_limitation() {
    assert!(
        STRDIST_VR.contains("grapheme") || STRDIST_VR.contains("graphemes"),
        "string_distance.vr must document the grapheme-cluster-aware limitation"
    );
}

// ── 3. pub fn levenshtein ─────────────────────────────────────────────────────

#[test]
fn string_distance_has_pub_fn_levenshtein() {
    assert!(
        STRDIST_VR.contains("pub fn levenshtein") || STRDIST_VR.contains("public fn levenshtein"),
        "string_distance.vr must export 'levenshtein'"
    );
}

// ── 4. pub fn levenshtein_bounded ────────────────────────────────────────────

#[test]
fn string_distance_has_pub_fn_levenshtein_bounded() {
    assert!(
        STRDIST_VR.contains("levenshtein_bounded"),
        "string_distance.vr must export 'levenshtein_bounded'"
    );
}

// ── 5. pub fn jaro ────────────────────────────────────────────────────────────

#[test]
fn string_distance_has_pub_fn_jaro() {
    assert!(
        STRDIST_VR.contains("pub fn jaro") || STRDIST_VR.contains("public fn jaro"),
        "string_distance.vr must export 'jaro'"
    );
}

// ── 6. pub fn jaro_winkler ────────────────────────────────────────────────────

#[test]
fn string_distance_has_pub_fn_jaro_winkler() {
    assert!(
        STRDIST_VR.contains("jaro_winkler"),
        "string_distance.vr must export 'jaro_winkler'"
    );
}

// ── 7. pub fn closest ────────────────────────────────────────────────────────

#[test]
fn string_distance_has_pub_fn_closest() {
    assert!(
        STRDIST_VR.contains("pub fn closest") || STRDIST_VR.contains("public fn closest"),
        "string_distance.vr must export 'closest'"
    );
}

// ── 8. levenshtein takes &[Byte] ─────────────────────────────────────────────

#[test]
fn levenshtein_takes_byte_slice_params() {
    assert!(
        STRDIST_VR.contains("&[Byte]"),
        "levenshtein must take '&[Byte]' slice parameters"
    );
}

// ── 9. VCS spec ───────────────────────────────────────────────────────────────

#[test]
fn strdist_spec_uses_levenshtein() {
    assert!(
        STRDIST_SPEC.contains("levenshtein("),
        "string_distance_unicode.vr must call 'levenshtein('"
    );
}

#[test]
fn strdist_spec_uses_levenshtein_bounded() {
    assert!(
        STRDIST_SPEC.contains("levenshtein_bounded("),
        "string_distance_unicode.vr must call 'levenshtein_bounded('"
    );
}

#[test]
fn strdist_spec_uses_jaro() {
    assert!(
        STRDIST_SPEC.contains("jaro("),
        "string_distance_unicode.vr must call 'jaro('"
    );
}

#[test]
fn strdist_spec_uses_jaro_winkler() {
    assert!(
        STRDIST_SPEC.contains("jaro_winkler("),
        "string_distance_unicode.vr must call 'jaro_winkler('"
    );
}

#[test]
fn strdist_spec_uses_closest() {
    assert!(
        STRDIST_SPEC.contains("closest("),
        "string_distance_unicode.vr must call 'closest('"
    );
}

#[test]
fn strdist_spec_is_typecheck_pass() {
    assert!(
        STRDIST_SPEC.contains("@test: typecheck-pass"),
        "string_distance_unicode.vr must be '@test: typecheck-pass'"
    );
}
