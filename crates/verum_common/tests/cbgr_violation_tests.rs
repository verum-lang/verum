//! Tests for CbgrViolationKind and CbgrViolation types.
//!
//! Tests for the CBGR (Capability-Based Generation References) violation system.
//! CbgrViolationKind is the single source of truth for all memory safety violations
//! across all execution tiers. FFI error codes are in the 0x1001-0x1008 range.

use verum_common::{CbgrViolation, CbgrViolationKind, ErrorKind, VerumError};

// =============================================================================
// CbgrViolationKind Tests
// =============================================================================

#[test]
fn test_violation_kind_is_fatal() {
    // Fatal violations indicate memory corruption
    assert!(CbgrViolationKind::UseAfterFree.is_fatal());
    assert!(CbgrViolationKind::DoubleFree.is_fatal());
    assert!(CbgrViolationKind::NullPointer.is_fatal());
    assert!(CbgrViolationKind::InvalidReference.is_fatal());

    // Non-fatal violations
    assert!(!CbgrViolationKind::GenerationMismatch.is_fatal());
    assert!(!CbgrViolationKind::EpochExpired.is_fatal());
    assert!(!CbgrViolationKind::CapabilityDenied.is_fatal());
    assert!(!CbgrViolationKind::OutOfBounds.is_fatal());
}

#[test]
fn test_violation_kind_is_recoverable() {
    // Recoverable violations (policy, not corruption)
    assert!(CbgrViolationKind::CapabilityDenied.is_recoverable());
    assert!(CbgrViolationKind::OutOfBounds.is_recoverable());

    // Non-recoverable violations
    assert!(!CbgrViolationKind::UseAfterFree.is_recoverable());
    assert!(!CbgrViolationKind::DoubleFree.is_recoverable());
    assert!(!CbgrViolationKind::GenerationMismatch.is_recoverable());
    assert!(!CbgrViolationKind::EpochExpired.is_recoverable());
    assert!(!CbgrViolationKind::NullPointer.is_recoverable());
    assert!(!CbgrViolationKind::InvalidReference.is_recoverable());
}

#[test]
fn test_violation_kind_ffi_error_codes() {
    // Verify error codes are in expected range 0x1001-0x1008
    assert_eq!(CbgrViolationKind::UseAfterFree.ffi_error_code(), 0x1001);
    assert_eq!(CbgrViolationKind::DoubleFree.ffi_error_code(), 0x1002);
    assert_eq!(CbgrViolationKind::GenerationMismatch.ffi_error_code(), 0x1003);
    assert_eq!(CbgrViolationKind::EpochExpired.ffi_error_code(), 0x1004);
    assert_eq!(CbgrViolationKind::CapabilityDenied.ffi_error_code(), 0x1005);
    assert_eq!(CbgrViolationKind::InvalidReference.ffi_error_code(), 0x1006);
    assert_eq!(CbgrViolationKind::NullPointer.ffi_error_code(), 0x1007);
    assert_eq!(CbgrViolationKind::OutOfBounds.ffi_error_code(), 0x1008);
}

#[test]
fn test_violation_kind_from_ffi_error_code() {
    // Valid codes
    assert_eq!(
        CbgrViolationKind::from_ffi_error_code(0x1001),
        Some(CbgrViolationKind::UseAfterFree)
    );
    assert_eq!(
        CbgrViolationKind::from_ffi_error_code(0x1002),
        Some(CbgrViolationKind::DoubleFree)
    );
    assert_eq!(
        CbgrViolationKind::from_ffi_error_code(0x1003),
        Some(CbgrViolationKind::GenerationMismatch)
    );
    assert_eq!(
        CbgrViolationKind::from_ffi_error_code(0x1004),
        Some(CbgrViolationKind::EpochExpired)
    );
    assert_eq!(
        CbgrViolationKind::from_ffi_error_code(0x1005),
        Some(CbgrViolationKind::CapabilityDenied)
    );
    assert_eq!(
        CbgrViolationKind::from_ffi_error_code(0x1006),
        Some(CbgrViolationKind::InvalidReference)
    );
    assert_eq!(
        CbgrViolationKind::from_ffi_error_code(0x1007),
        Some(CbgrViolationKind::NullPointer)
    );
    assert_eq!(
        CbgrViolationKind::from_ffi_error_code(0x1008),
        Some(CbgrViolationKind::OutOfBounds)
    );

    // Invalid codes
    assert_eq!(CbgrViolationKind::from_ffi_error_code(0x1000), None);
    assert_eq!(CbgrViolationKind::from_ffi_error_code(0x1009), None);
    assert_eq!(CbgrViolationKind::from_ffi_error_code(0), None);
    assert_eq!(CbgrViolationKind::from_ffi_error_code(0x2000), None);
}

#[test]
fn test_violation_kind_roundtrip_ffi_code() {
    // Every kind should roundtrip through FFI code
    let kinds = [
        CbgrViolationKind::UseAfterFree,
        CbgrViolationKind::DoubleFree,
        CbgrViolationKind::GenerationMismatch,
        CbgrViolationKind::EpochExpired,
        CbgrViolationKind::CapabilityDenied,
        CbgrViolationKind::InvalidReference,
        CbgrViolationKind::NullPointer,
        CbgrViolationKind::OutOfBounds,
    ];

    for kind in kinds {
        let code = kind.ffi_error_code();
        let recovered = CbgrViolationKind::from_ffi_error_code(code);
        assert_eq!(recovered, Some(kind), "Failed roundtrip for {:?}", kind);
    }
}

#[test]
fn test_violation_kind_description() {
    // Each kind should have a non-empty description
    let kinds = [
        CbgrViolationKind::UseAfterFree,
        CbgrViolationKind::DoubleFree,
        CbgrViolationKind::GenerationMismatch,
        CbgrViolationKind::EpochExpired,
        CbgrViolationKind::CapabilityDenied,
        CbgrViolationKind::InvalidReference,
        CbgrViolationKind::NullPointer,
        CbgrViolationKind::OutOfBounds,
    ];

    for kind in kinds {
        let desc = kind.description();
        assert!(!desc.is_empty(), "Empty description for {:?}", kind);
        // Description should contain a colon separator
        assert!(
            desc.contains(':'),
            "Description should be 'name: detail' format for {:?}",
            kind
        );
    }
}

#[test]
fn test_violation_kind_name() {
    assert_eq!(CbgrViolationKind::UseAfterFree.name(), "UseAfterFree");
    assert_eq!(CbgrViolationKind::DoubleFree.name(), "DoubleFree");
    assert_eq!(CbgrViolationKind::GenerationMismatch.name(), "GenerationMismatch");
    assert_eq!(CbgrViolationKind::EpochExpired.name(), "EpochExpired");
    assert_eq!(CbgrViolationKind::CapabilityDenied.name(), "CapabilityDenied");
    assert_eq!(CbgrViolationKind::InvalidReference.name(), "InvalidReference");
    assert_eq!(CbgrViolationKind::NullPointer.name(), "NullPointer");
    assert_eq!(CbgrViolationKind::OutOfBounds.name(), "OutOfBounds");
}

#[test]
fn test_violation_kind_display() {
    // Display should match description
    for kind in [
        CbgrViolationKind::UseAfterFree,
        CbgrViolationKind::DoubleFree,
    ] {
        assert_eq!(format!("{}", kind), kind.description());
    }
}

#[test]
fn test_violation_kind_copy_clone() {
    // Ensure Copy semantics work correctly
    let kind = CbgrViolationKind::UseAfterFree;
    let copied = kind;
    let cloned = kind;
    assert_eq!(kind, copied);
    assert_eq!(kind, cloned);
}

#[test]
fn test_violation_kind_hash() {
    use std::collections::HashSet;

    let mut set = HashSet::new();
    set.insert(CbgrViolationKind::UseAfterFree);
    set.insert(CbgrViolationKind::DoubleFree);
    set.insert(CbgrViolationKind::UseAfterFree); // Duplicate

    assert_eq!(set.len(), 2);
    assert!(set.contains(&CbgrViolationKind::UseAfterFree));
    assert!(set.contains(&CbgrViolationKind::DoubleFree));
}

// =============================================================================
// CbgrViolation Tests
// =============================================================================

#[test]
fn test_violation_new() {
    let violation = CbgrViolation::new(CbgrViolationKind::UseAfterFree, 0xDEADBEEF);

    assert_eq!(violation.kind, CbgrViolationKind::UseAfterFree);
    assert_eq!(violation.pointer, 0xDEADBEEF);
    assert!(violation.expected_generation.is_none());
    assert!(violation.actual_generation.is_none());
    assert!(violation.expected_epoch.is_none());
    assert!(violation.actual_epoch.is_none());
    assert!(violation.type_name.is_none());
}

#[test]
fn test_violation_with_generation() {
    let violation = CbgrViolation::new(CbgrViolationKind::GenerationMismatch, 0x1000)
        .with_generation(42, 100);

    assert_eq!(violation.expected_generation, Some(42));
    assert_eq!(violation.actual_generation, Some(100));
}

#[test]
fn test_violation_with_epoch() {
    let violation = CbgrViolation::new(CbgrViolationKind::EpochExpired, 0x2000).with_epoch(1, 5);

    assert_eq!(violation.expected_epoch, Some(1));
    assert_eq!(violation.actual_epoch, Some(5));
}

#[test]
fn test_violation_with_type_name() {
    let violation =
        CbgrViolation::new(CbgrViolationKind::UseAfterFree, 0x3000).with_type_name("MyStruct");

    assert_eq!(violation.type_name.as_ref().map(|s| s.as_str()), Some("MyStruct"));
}

#[test]
fn test_violation_builder_chaining() {
    let violation = CbgrViolation::new(CbgrViolationKind::UseAfterFree, 0xDEADBEEF)
        .with_generation(42, 100)
        .with_epoch(1, 2)
        .with_type_name("TestType");

    assert_eq!(violation.kind, CbgrViolationKind::UseAfterFree);
    assert_eq!(violation.pointer, 0xDEADBEEF);
    assert_eq!(violation.expected_generation, Some(42));
    assert_eq!(violation.actual_generation, Some(100));
    assert_eq!(violation.expected_epoch, Some(1));
    assert_eq!(violation.actual_epoch, Some(2));
    assert_eq!(violation.type_name.as_ref().map(|s| s.as_str()), Some("TestType"));
}

#[test]
fn test_violation_display() {
    let violation = CbgrViolation::new(CbgrViolationKind::UseAfterFree, 0x1234);
    let display = format!("{}", violation);

    assert!(display.contains("CBGR violation"));
    assert!(display.contains("UseAfterFree"));
    assert!(display.contains("0x"));
}

#[test]
fn test_violation_display_with_context() {
    let violation = CbgrViolation::new(CbgrViolationKind::GenerationMismatch, 0x5678)
        .with_generation(10, 20)
        .with_epoch(1, 2)
        .with_type_name("Node<i32>");

    let display = format!("{}", violation);

    assert!(display.contains("GenerationMismatch"));
    assert!(display.contains("gen: expected=10, actual=20"));
    assert!(display.contains("epoch: expected=1, actual=2"));
    assert!(display.contains("[type: Node<i32>]"));
}

#[test]
fn test_violation_to_error() {
    let violation = CbgrViolation::new(CbgrViolationKind::UseAfterFree, 0x1000);
    let error = violation.to_error();

    assert_eq!(error.kind(), ErrorKind::Cbgr);
    assert!(error.message().as_str().contains("UseAfterFree"));
}

#[test]
fn test_violation_into_error() {
    let violation = CbgrViolation::new(CbgrViolationKind::DoubleFree, 0x2000);
    let error: VerumError = violation.into();

    assert_eq!(error.kind(), ErrorKind::Cbgr);
}

#[test]
fn test_violation_kind_into_error() {
    let error: VerumError = CbgrViolationKind::NullPointer.into();

    assert_eq!(error.kind(), ErrorKind::Cbgr);
    assert!(error.message().as_str().contains("null pointer"));
}

#[test]
fn test_violation_equality() {
    let v1 = CbgrViolation::new(CbgrViolationKind::UseAfterFree, 0x1000).with_generation(1, 2);
    let v2 = CbgrViolation::new(CbgrViolationKind::UseAfterFree, 0x1000).with_generation(1, 2);
    let v3 = CbgrViolation::new(CbgrViolationKind::UseAfterFree, 0x1000).with_generation(1, 3);

    assert_eq!(v1, v2);
    assert_ne!(v1, v3);
}

#[test]
fn test_violation_clone() {
    let original = CbgrViolation::new(CbgrViolationKind::OutOfBounds, 0x3000)
        .with_type_name("Vec<u8>");

    let cloned = original.clone();

    assert_eq!(original, cloned);
}

// =============================================================================
// Integration with ErrorKind Tests
// =============================================================================

#[test]
fn test_error_kind_cbgr_level() {
    // CBGR errors should map to a specific level in the error hierarchy
    // CBGR is a domain-specific error kind that defaults to level 2 (runtime)
    // that defaults to level 2 (runtime)
    let err = VerumError::cbgr("test error");
    assert_eq!(err.kind(), ErrorKind::Cbgr);
}

#[test]
fn test_cbgr_specific_constructors() {
    // Test the specific CBGR constructors on VerumError
    // Note: use_after_free requires matching epochs to trigger the "use-after-free" message
    let err = VerumError::use_after_free(42, 100, 1, 1, "TestType", 0);
    assert_eq!(err.kind(), ErrorKind::Cbgr);
    assert!(err.message().as_str().contains("use-after-free"), "Expected 'use-after-free' in: {}", err.message());

    // Test null pointer case (gen == gen_unallocated)
    let err = VerumError::use_after_free(0, 100, 1, 1, "NullTest", 0);
    assert_eq!(err.kind(), ErrorKind::Cbgr);
    assert!(err.message().as_str().contains("null pointer"), "Expected 'null pointer' in: {}", err.message());

    // Test epoch mismatch case (epoch1 != epoch2)
    let err = VerumError::use_after_free(42, 100, 1, 2, "EpochTest", 0);
    assert_eq!(err.kind(), ErrorKind::Cbgr);
    assert!(err.message().as_str().contains("epoch mismatch"), "Expected 'epoch mismatch' in: {}", err.message());

    let err = VerumError::generation_mismatch(10, 20, "Node");
    assert_eq!(err.kind(), ErrorKind::Cbgr);
    assert!(err.message().as_str().contains("generation mismatch"));

    let err = VerumError::epoch_mismatch(1, 5, "Buffer");
    assert_eq!(err.kind(), ErrorKind::Cbgr);
    assert!(err.message().as_str().contains("epoch mismatch"));
}

// =============================================================================
// Property Tests
// =============================================================================

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;

    fn arb_cbgr_violation_kind() -> impl Strategy<Value = CbgrViolationKind> {
        prop_oneof![
            Just(CbgrViolationKind::UseAfterFree),
            Just(CbgrViolationKind::DoubleFree),
            Just(CbgrViolationKind::GenerationMismatch),
            Just(CbgrViolationKind::EpochExpired),
            Just(CbgrViolationKind::CapabilityDenied),
            Just(CbgrViolationKind::InvalidReference),
            Just(CbgrViolationKind::NullPointer),
            Just(CbgrViolationKind::OutOfBounds),
        ]
    }

    proptest! {
        #[test]
        fn ffi_code_roundtrip(kind in arb_cbgr_violation_kind()) {
            let code = kind.ffi_error_code();
            let recovered = CbgrViolationKind::from_ffi_error_code(code);
            prop_assert_eq!(recovered, Some(kind));
        }

        #[test]
        fn ffi_codes_in_valid_range(kind in arb_cbgr_violation_kind()) {
            let code = kind.ffi_error_code();
            prop_assert!((0x1001..=0x1008).contains(&code));
        }

        #[test]
        fn description_not_empty(kind in arb_cbgr_violation_kind()) {
            prop_assert!(!kind.description().is_empty());
        }

        #[test]
        fn name_not_empty(kind in arb_cbgr_violation_kind()) {
            prop_assert!(!kind.name().is_empty());
        }

        #[test]
        fn violation_display_contains_kind_name(
            kind in arb_cbgr_violation_kind(),
            pointer in any::<usize>()
        ) {
            let violation = CbgrViolation::new(kind, pointer);
            let display = format!("{}", violation);
            prop_assert!(display.contains(kind.name()));
        }
    }
}
