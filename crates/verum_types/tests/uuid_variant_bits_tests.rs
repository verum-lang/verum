#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! UUID variant-bits RFC 4122 drift guard (#74).
//!
//! `core/base/uuid.vr` generates UUIDs with correct variant (byte 8 top two
//! bits = 10xx) and version stamps (byte 6 top four bits = version nibble).
//!
//! This drift guard pins:
//!   1. uuid.vr stamps version 4 as 0x40 in byte 6.
//!   2. uuid.vr stamps version 7 as 0x70 in byte 6.
//!   3. uuid.vr stamps RFC variant as 0x80 in byte 8.
//!   4. uuid.vr preserves low 6 bits of byte 8 via 0x3F mask.
//!   5. uuid.vr defines UuidError with position and message.
//!   6. uuid.vr has `pub fn parse` method on Uuid.
//!   7. uuid.vr has `version` method on Uuid.
//!   8. uuid.vr has `nil` constructor.
//!   9. uuid.vr has `to_text` method (36-char hyphenated format).
//!  10. VCS spec uses UuidError, Uuid.parse, Uuid.nil.

const UUID_VR: &str = include_str!("../../../core/base/uuid.vr");
const UUID_SPEC: &str = include_str!(
    "../../../vcs/specs/L2-standard/time/uuid_variant_bits_rfc4122.vr"
);

// ── 1. version 4 stamp: byte6 | 0x40 ────────────────────────────────────────

#[test]
fn uuid_vr_stamps_version_4_as_0x40() {
    assert!(
        UUID_VR.contains("0x40_u8"),
        "uuid.vr must stamp version 4 with 0x40_u8 in byte 6"
    );
}

// ── 2. version 7 stamp: byte6 | 0x70 ────────────────────────────────────────

#[test]
fn uuid_vr_stamps_version_7_as_0x70() {
    assert!(
        UUID_VR.contains("0x70_u8"),
        "uuid.vr must stamp version 7 with 0x70_u8 in byte 6"
    );
}

// ── 3. RFC variant stamp: byte8 | 0x80 ──────────────────────────────────────

#[test]
fn uuid_vr_stamps_rfc_variant_as_0x80() {
    assert!(
        UUID_VR.contains("0x80_u8"),
        "uuid.vr must stamp RFC variant with 0x80_u8 in byte 8"
    );
}

// ── 4. preserve low 6 bits of byte8 via 0x3F ────────────────────────────────

#[test]
fn uuid_vr_preserves_low_6_bits_with_0x3f() {
    assert!(
        UUID_VR.contains("0x3F_u8"),
        "uuid.vr must preserve low 6 bits of byte 8 using 0x3F_u8 mask"
    );
}

// ── 5. UuidError with position and message ───────────────────────────────────

#[test]
fn uuid_error_has_position_field() {
    assert!(
        UUID_VR.contains("position:"),
        "UuidError must have a 'position' field"
    );
}

#[test]
fn uuid_error_has_message_field() {
    assert!(
        UUID_VR.contains("message:"),
        "UuidError must have a 'message' field"
    );
}

// ── 6. parse method ───────────────────────────────────────────────────────────

#[test]
fn uuid_vr_has_parse_method() {
    assert!(
        UUID_VR.contains("fn parse"),
        "Uuid must have a 'parse' method"
    );
}

// ── 7. version method ─────────────────────────────────────────────────────────

#[test]
fn uuid_vr_has_version_method() {
    assert!(
        UUID_VR.contains("fn version"),
        "Uuid must have a 'version' method"
    );
}

// ── 8. nil constructor ────────────────────────────────────────────────────────

#[test]
fn uuid_vr_has_nil_constructor() {
    assert!(
        UUID_VR.contains("fn nil"),
        "Uuid must have a 'nil' constructor"
    );
}

// ── 9. to_text method ─────────────────────────────────────────────────────────

#[test]
fn uuid_vr_has_to_text_method() {
    assert!(
        UUID_VR.contains("fn to_text"),
        "Uuid must have a 'to_text' method"
    );
}

// ── 10. VCS spec ──────────────────────────────────────────────────────────────

#[test]
fn uuid_spec_uses_uuid_error() {
    assert!(
        UUID_SPEC.contains("UuidError"),
        "uuid_variant_bits_rfc4122.vr must reference UuidError"
    );
}

#[test]
fn uuid_spec_uses_uuid_parse() {
    assert!(
        UUID_SPEC.contains("Uuid.parse"),
        "uuid_variant_bits_rfc4122.vr must use 'Uuid.parse'"
    );
}

#[test]
fn uuid_spec_uses_uuid_nil() {
    assert!(
        UUID_SPEC.contains("Uuid.nil"),
        "uuid_variant_bits_rfc4122.vr must use 'Uuid.nil'"
    );
}

#[test]
fn uuid_spec_is_typecheck_pass() {
    assert!(
        UUID_SPEC.contains("@test: typecheck-pass"),
        "uuid_variant_bits_rfc4122.vr must be '@test: typecheck-pass'"
    );
}
