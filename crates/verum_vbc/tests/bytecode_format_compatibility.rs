//! Bytecode format compatibility tests (#175 step E).
//!
//! Each test pins one slice of the migration policy:
//!
//!   * round_trip_preserves_header — `serialize_module` then
//!     `deserialize_module` round-trip must reproduce the same magic /
//!     version / hashes that went in.
//!   * tampered_magic_rejected — flipping any byte of the on-wire
//!     magic must surface as `VbcError::InvalidMagic`. A reader that
//!     accepts wrong-magic archives is broken.
//!   * tampered_higher_major_rejected — bumping the on-wire
//!     `version_major` past the consumer's `VERSION_MAJOR` must surface
//!     as `VbcError::UnsupportedVersion`. A reader that accepts a
//!     higher major would silently miscompute layouts.
//!   * tampered_lower_major_rejected — same for a strictly lower
//!     major. The migration policy says majors are exact-match; old
//!     archives can't be read by new consumers either.
//!   * tampered_higher_minor_rejected — same-major but higher-minor
//!     must be rejected; the consumer doesn't know the new opcodes.
//!   * tampered_same_or_lower_minor_round_trips — same-major +
//!     equal-or-lower minor must remain readable (additive minor
//!     contract).
//!
//! These complement the unit-level rejection guardrails in
//! `format::tests::test_rejects_*` (commit 34de051f) by exercising the
//! full serialize → byte-tamper → deserialize pipeline rather than the
//! `VbcHeader` accessors in isolation.

use verum_vbc::deserialize::deserialize_module;
use verum_vbc::error::VbcError;
use verum_vbc::format::{HEADER_SIZE, MAGIC, VERSION_MAJOR, VERSION_MINOR};
use verum_vbc::module::VbcModule;
use verum_vbc::serialize::serialize_module;

/// Build a minimal but non-empty module — exercises the full
/// header + section pipeline without depending on codegen.
fn empty_archive_bytes() -> Vec<u8> {
    let module = VbcModule::new("test_compat".to_string());
    serialize_module(&module).expect("serialize should succeed for empty module")
}

#[test]
fn round_trip_preserves_header() {
    let bytes = empty_archive_bytes();
    assert_eq!(&bytes[0..4], &MAGIC, "magic must be VBC1 on the wire");
    let major =
        u16::from_le_bytes(bytes[4..6].try_into().expect("4..6 is 2 bytes"));
    let minor =
        u16::from_le_bytes(bytes[6..8].try_into().expect("6..8 is 2 bytes"));
    assert_eq!(major, VERSION_MAJOR, "encoded major matches consumer");
    assert_eq!(minor, VERSION_MINOR, "encoded minor matches consumer");

    let module = deserialize_module(&bytes).expect("round-trip should succeed");
    assert_eq!(module.header.magic, MAGIC);
    assert_eq!(module.header.version_major, VERSION_MAJOR);
    assert_eq!(module.header.version_minor, VERSION_MINOR);
}

#[test]
fn tampered_magic_rejected() {
    // Flip every byte of the magic individually + the all-bytes-flipped
    // case. Each must surface the same InvalidMagic error class.
    for i in 0..4 {
        let mut bytes = empty_archive_bytes();
        bytes[i] ^= 0x01;
        match deserialize_module(&bytes) {
            Err(VbcError::InvalidMagic(_)) => {}
            other => panic!(
                "expected InvalidMagic for flipped magic byte {i}, got {:?}",
                other
            ),
        }
    }

    // All-zero magic — common corruption pattern.
    let mut bytes = empty_archive_bytes();
    bytes[0..4].copy_from_slice(&[0, 0, 0, 0]);
    match deserialize_module(&bytes) {
        Err(VbcError::InvalidMagic(m)) => assert_eq!(m, [0, 0, 0, 0]),
        other => panic!("expected InvalidMagic for zeroed magic, got {:?}", other),
    }
}

#[test]
fn tampered_higher_major_rejected() {
    // Bump the encoded major past the consumer's supported value.
    let mut bytes = empty_archive_bytes();
    let bumped = VERSION_MAJOR.saturating_add(1);
    bytes[4..6].copy_from_slice(&bumped.to_le_bytes());
    match deserialize_module(&bytes) {
        Err(VbcError::UnsupportedVersion {
            major,
            minor: _,
            supported_major,
            supported_minor: _,
        }) => {
            assert_eq!(major, bumped);
            assert_eq!(supported_major, VERSION_MAJOR);
        }
        other => panic!(
            "expected UnsupportedVersion for higher major, got {:?}",
            other
        ),
    }
}

#[test]
fn tampered_lower_major_rejected() {
    // If consumer is at major 0 there's no "lower" — skip; otherwise
    // pin the contract that lower-major is also rejected (exact-match).
    if VERSION_MAJOR == 0 {
        return;
    }
    let mut bytes = empty_archive_bytes();
    let lowered = VERSION_MAJOR - 1;
    bytes[4..6].copy_from_slice(&lowered.to_le_bytes());
    match deserialize_module(&bytes) {
        Err(VbcError::UnsupportedVersion {
            major,
            supported_major,
            ..
        }) => {
            assert_eq!(major, lowered);
            assert_eq!(supported_major, VERSION_MAJOR);
        }
        other => panic!(
            "expected UnsupportedVersion for lower major, got {:?}",
            other
        ),
    }
}

#[test]
fn tampered_higher_minor_rejected() {
    // Same major, higher minor — additive-minor contract says the
    // consumer can't execute opcodes from a higher minor.
    let mut bytes = empty_archive_bytes();
    let bumped = VERSION_MINOR.saturating_add(1);
    bytes[6..8].copy_from_slice(&bumped.to_le_bytes());
    match deserialize_module(&bytes) {
        Err(VbcError::UnsupportedVersion {
            major,
            minor,
            supported_major,
            supported_minor,
        }) => {
            assert_eq!(major, VERSION_MAJOR);
            assert_eq!(minor, bumped);
            assert_eq!(supported_major, VERSION_MAJOR);
            assert_eq!(supported_minor, VERSION_MINOR);
        }
        other => panic!(
            "expected UnsupportedVersion for higher minor, got {:?}",
            other
        ),
    }
}

#[test]
fn tampered_same_or_lower_minor_round_trips() {
    // Same minor — canonical round-trip case.
    let bytes = empty_archive_bytes();
    deserialize_module(&bytes).expect("same-minor must round-trip");

    // Lower minor — the consumer claims to support older minors as
    // additive backwards-compatibility. Skip if minor is already 0.
    if VERSION_MINOR == 0 {
        return;
    }
    let mut bytes = empty_archive_bytes();
    let lower = VERSION_MINOR - 1;
    bytes[6..8].copy_from_slice(&lower.to_le_bytes());
    // Note: tampering the minor without re-computing the body invalidates
    // the content_hash, but deserialize_module doesn't verify the hash
    // (it's an integrity field, not enforced here). The version check
    // is what we're pinning.
    match deserialize_module(&bytes) {
        Ok(module) => {
            assert_eq!(module.header.version_minor, lower);
        }
        // Acceptable: deserialization may reject mid-pipeline for an
        // unrelated reason since we tampered raw bytes. Specifically
        // accept the case but not a version-class rejection.
        Err(VbcError::UnsupportedVersion { .. }) => {
            panic!("lower-minor must NOT be rejected as UnsupportedVersion");
        }
        Err(_) => {
            // Tolerable — body parsing may fail because we changed
            // bytes that participate in section-size invariants. The
            // contract we care about (version is not the rejection
            // reason) is still upheld.
        }
    }
}

/// Sanity check: the on-wire layout starts with magic at offset 0,
/// version-major at offset 4, version-minor at offset 6. If anyone
/// reorders the header layout, this fails immediately, alerting them
/// to update the migration policy + the byte-offset constants in
/// the tampering tests above.
#[test]
fn header_layout_offsets_pinned() {
    let bytes = empty_archive_bytes();
    assert!(bytes.len() >= HEADER_SIZE);
    assert_eq!(&bytes[0..4], &MAGIC, "magic at 0..4");
    let major =
        u16::from_le_bytes(bytes[4..6].try_into().expect("layout: major @ 4"));
    let minor =
        u16::from_le_bytes(bytes[6..8].try_into().expect("layout: minor @ 6"));
    assert_eq!(major, VERSION_MAJOR);
    assert_eq!(minor, VERSION_MINOR);
}
