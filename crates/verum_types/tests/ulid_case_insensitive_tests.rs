#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! ULID case-insensitive parse normalisation drift guard (#73).
//!
//! `core/base/ulid.vr` implements the ULID parser with Crockford Base32
//! case normalisation: lowercase input is converted to uppercase before
//! the alphabet lookup, and the aliases I/L → 1, O → 0 are accepted.
//!
//! This drift guard pins:
//!   1. ulid.vr defines `ULID_ALPHABET` with 32 entries.
//!   2. ulid.vr has `decode_char` that converts lowercase to uppercase.
//!   3. decode_char handles the Crockford aliases I/L → 1 and O → 0.
//!   4. ulid.vr defines `UlidError` with InvalidLength and InvalidCharacter.
//!   5. ulid.vr has `pub fn parse` function.
//!   6. ulid.vr has `timestamp_ms` method on Ulid.
//!   7. ulid.vr has `to_text` method on Ulid.
//!   8. VCS spec uses `parse`, `UlidError`, `ULID_ALPHABET`.

const ULID_VR: &str = include_str!("../../../core/base/ulid.vr");
const ULID_SPEC: &str = include_str!(
    "../../../vcs/specs/L2-standard/time/ulid_case_insensitive_parse.vr"
);

// ── 1. ULID_ALPHABET with 32 entries ─────────────────────────────────────────

#[test]
fn ulid_vr_defines_ulid_alphabet() {
    assert!(
        ULID_VR.contains("ULID_ALPHABET"),
        "ulid.vr must define ULID_ALPHABET"
    );
}

#[test]
fn ulid_alphabet_has_32_entries() {
    assert!(
        ULID_VR.contains("[Byte; 32]"),
        "ULID_ALPHABET must be typed as [Byte; 32] with 32 entries"
    );
}

// ── 2. decode_char normalises lowercase ──────────────────────────────────────

#[test]
fn decode_char_converts_lowercase_to_uppercase() {
    assert!(
        ULID_VR.contains("('a' as Byte)") && ULID_VR.contains("('A' as Byte)"),
        "decode_char must convert lowercase chars to uppercase by subtracting 'a'-'A'"
    );
}

// ── 3. Crockford aliases I/L → 1, O → 0 ─────────────────────────────────────

#[test]
fn decode_char_handles_i_and_l_alias() {
    assert!(
        ULID_VR.contains("('I' as Byte)") && ULID_VR.contains("('L' as Byte)"),
        "decode_char must handle I/L aliases mapping to 1"
    );
}

#[test]
fn decode_char_handles_o_alias() {
    assert!(
        ULID_VR.contains("('O' as Byte)"),
        "decode_char must handle O alias mapping to 0"
    );
}

// ── 4. UlidError variants ─────────────────────────────────────────────────────

#[test]
fn ulid_error_has_invalid_length() {
    assert!(
        ULID_VR.contains("InvalidLength"),
        "UlidError must have InvalidLength variant"
    );
}

#[test]
fn ulid_error_has_invalid_character() {
    assert!(
        ULID_VR.contains("InvalidCharacter"),
        "UlidError must have InvalidCharacter variant"
    );
}

// ── 5. pub fn parse ───────────────────────────────────────────────────────────

#[test]
fn ulid_vr_has_pub_fn_parse() {
    assert!(
        ULID_VR.contains("public fn parse") || ULID_VR.contains("pub fn parse"),
        "ulid.vr must export a 'parse' function"
    );
}

// ── 6. timestamp_ms method ────────────────────────────────────────────────────

#[test]
fn ulid_vr_has_timestamp_ms() {
    assert!(
        ULID_VR.contains("fn timestamp_ms"),
        "Ulid must have a 'timestamp_ms' method"
    );
}

// ── 7. to_text method ─────────────────────────────────────────────────────────

#[test]
fn ulid_vr_has_to_text() {
    assert!(
        ULID_VR.contains("fn to_text"),
        "Ulid must have a 'to_text' method"
    );
}

// ── 8. VCS spec ───────────────────────────────────────────────────────────────

#[test]
fn ulid_spec_uses_parse() {
    assert!(
        ULID_SPEC.contains("parse("),
        "ulid_case_insensitive_parse.vr must call 'parse('"
    );
}

#[test]
fn ulid_spec_uses_ulid_error() {
    assert!(
        ULID_SPEC.contains("UlidError"),
        "ulid_case_insensitive_parse.vr must reference UlidError"
    );
}

#[test]
fn ulid_spec_uses_ulid_alphabet() {
    assert!(
        ULID_SPEC.contains("ULID_ALPHABET"),
        "ulid_case_insensitive_parse.vr must reference ULID_ALPHABET"
    );
}

#[test]
fn ulid_spec_is_typecheck_pass() {
    assert!(
        ULID_SPEC.contains("@test: typecheck-pass"),
        "ulid_case_insensitive_parse.vr must be '@test: typecheck-pass'"
    );
}
