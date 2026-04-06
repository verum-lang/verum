#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
//! Tests for VBC type system (TypeId, TypeRef) and CBGR reference types
//! (ThinRef, FatRef, Capabilities).

use verum_vbc::types::{TypeId, TypeRef, StringId, TypeParamId};
use verum_vbc::value::{Capabilities, ThinRef, FatRef};

// ============================================================================
// TypeId Tests
// ============================================================================

#[test]
fn test_builtin_type_ids() {
    assert_eq!(TypeId::UNIT.0, 0);
    assert_eq!(TypeId::BOOL.0, 1);
    assert_eq!(TypeId::INT.0, 2);
    assert_eq!(TypeId::FLOAT.0, 3);
    assert_eq!(TypeId::TEXT.0, 4);
    assert_eq!(TypeId::NEVER.0, 5);
}

#[test]
fn test_type_id_aliases() {
    assert_eq!(TypeId::I64, TypeId::INT);
    assert_eq!(TypeId::F64, TypeId::FLOAT);
    assert_eq!(TypeId::ISIZE, TypeId::PTR);
    assert_eq!(TypeId::USIZE, TypeId::PTR);
}

#[test]
fn test_is_builtin() {
    assert!(TypeId::UNIT.is_builtin());
    assert!(TypeId::BOOL.is_builtin());
    assert!(TypeId::INT.is_builtin());
    assert!(TypeId::FLOAT.is_builtin());
    assert!(TypeId::TEXT.is_builtin());
    assert!(TypeId::NEVER.is_builtin());
    assert!(TypeId::PTR.is_builtin());
    assert!(!TypeId(16).is_builtin()); // First user type
    assert!(!TypeId::LIST.is_builtin());
}

#[test]
fn test_is_numeric() {
    assert!(TypeId::INT.is_numeric());
    assert!(TypeId::FLOAT.is_numeric());
    assert!(TypeId::U8.is_numeric());
    assert!(TypeId::U16.is_numeric());
    assert!(TypeId::U32.is_numeric());
    assert!(TypeId::U64.is_numeric());
    assert!(TypeId::I8.is_numeric());
    assert!(TypeId::I16.is_numeric());
    assert!(TypeId::I32.is_numeric());
    assert!(TypeId::F32.is_numeric());
    assert!(!TypeId::BOOL.is_numeric());
    assert!(!TypeId::TEXT.is_numeric());
    assert!(!TypeId::UNIT.is_numeric());
}

#[test]
fn test_is_integer() {
    assert!(TypeId::INT.is_integer());
    assert!(TypeId::U8.is_integer());
    assert!(TypeId::I8.is_integer());
    assert!(TypeId::U64.is_integer());
    assert!(!TypeId::FLOAT.is_integer());
    assert!(!TypeId::F32.is_integer());
    assert!(!TypeId::BOOL.is_integer());
}

#[test]
fn test_is_float() {
    assert!(TypeId::FLOAT.is_float());
    assert!(TypeId::F32.is_float());
    assert!(!TypeId::INT.is_float());
    assert!(!TypeId::U8.is_float());
}

#[test]
fn test_is_semantic_type() {
    assert!(TypeId::LIST.is_semantic_type());
    assert!(TypeId::MAP.is_semantic_type());
    assert!(TypeId::SET.is_semantic_type());
    assert!(TypeId::MAYBE.is_semantic_type());
    assert!(TypeId::RESULT.is_semantic_type());
    assert!(TypeId::RANGE.is_semantic_type());
    assert!(TypeId::ARRAY.is_semantic_type());
    assert!(TypeId::HEAP.is_semantic_type());
    assert!(TypeId::SHARED.is_semantic_type());
    assert!(TypeId::TUPLE.is_semantic_type());
    assert!(TypeId::DEQUE.is_semantic_type());
    assert!(TypeId::CHANNEL.is_semantic_type());
    assert!(!TypeId::INT.is_semantic_type());
    assert!(!TypeId::BOOL.is_semantic_type());
}

#[test]
fn test_is_iterable() {
    assert!(TypeId::LIST.is_iterable());
    assert!(TypeId::MAP.is_iterable());
    assert!(TypeId::SET.is_iterable());
    assert!(TypeId::RANGE.is_iterable());
    assert!(TypeId::ARRAY.is_iterable());
    assert!(!TypeId::MAYBE.is_iterable());
    assert!(!TypeId::RESULT.is_iterable());
    assert!(!TypeId::INT.is_iterable());
}

#[test]
fn test_is_meta_type() {
    assert!(TypeId::TOKEN_STREAM.is_meta_type());
    assert!(TypeId::TOKEN.is_meta_type());
    assert!(TypeId::TOKEN_KIND.is_meta_type());
    assert!(TypeId::SPAN.is_meta_type());
    assert!(!TypeId::INT.is_meta_type());
    assert!(!TypeId::LIST.is_meta_type());
}

// Note: TypeId::from_type_name does not exist yet in the current codebase.
// These tests cover the type ID constant values and their semantic groupings.

#[test]
fn test_semantic_type_id_ranges() {
    // Semantic types occupy range 512-1023
    assert_eq!(TypeId::FIRST_SEMANTIC, 512);
    assert_eq!(TypeId::LAST_SEMANTIC, 1023);
    assert!(TypeId::LIST.0 >= TypeId::FIRST_SEMANTIC);
    assert!(TypeId::CHANNEL.0 <= TypeId::LAST_SEMANTIC);
}

#[test]
fn test_first_user_type_id() {
    assert_eq!(TypeId::FIRST_USER, 16);
    assert!(TypeId::RESERVED.0 < TypeId::FIRST_USER);
}

#[test]
fn test_type_id_default() {
    assert_eq!(TypeId::default(), TypeId(0));
    assert_eq!(TypeId::default(), TypeId::UNIT);
}

// ============================================================================
// TypeRef Tests
// ============================================================================

#[test]
fn test_type_ref_concrete() {
    let tr = TypeRef::Concrete(TypeId::INT);
    assert_eq!(tr, TypeRef::Concrete(TypeId::INT));
}

#[test]
fn test_type_ref_generic() {
    let tr = TypeRef::Generic(TypeParamId(0));
    assert_eq!(tr, TypeRef::Generic(TypeParamId(0)));
}

#[test]
fn test_type_ref_instantiated() {
    let tr = TypeRef::Instantiated {
        base: TypeId::LIST,
        args: vec![TypeRef::Concrete(TypeId::INT)],
    };
    match &tr {
        TypeRef::Instantiated { base, args } => {
            assert_eq!(*base, TypeId::LIST);
            assert_eq!(args.len(), 1);
            assert_eq!(args[0], TypeRef::Concrete(TypeId::INT));
        }
        _ => panic!("Expected Instantiated"),
    }
}

// ============================================================================
// StringId Tests
// ============================================================================

#[test]
fn test_string_id_empty() {
    assert_eq!(StringId::EMPTY.0, 0);
}

#[test]
fn test_string_id_default() {
    assert_eq!(StringId::default(), StringId(0));
}

// ============================================================================
// Capabilities Tests
// ============================================================================

#[test]
fn test_capabilities_full() {
    let caps = Capabilities::FULL;
    assert!(caps.has(Capabilities::READ));
    assert!(caps.has(Capabilities::WRITE));
    assert!(caps.has(Capabilities::ADD));
    assert!(caps.has(Capabilities::REMOVE));
    assert!(caps.has(Capabilities::EXCLUSIVE));
    assert!(caps.has(Capabilities::DELEGATE));
    assert!(caps.has(Capabilities::ALIAS));
    assert!(caps.has(Capabilities::DROP));
}

#[test]
fn test_capabilities_read_only() {
    let caps = Capabilities::READ_ONLY;
    assert!(caps.has(Capabilities::READ));
    assert!(caps.has(Capabilities::ALIAS));
    assert!(!caps.has(Capabilities::WRITE));
    assert!(!caps.has(Capabilities::EXCLUSIVE));
    assert!(!caps.has(Capabilities::DROP));
}

#[test]
fn test_capabilities_mut_exclusive() {
    let caps = Capabilities::MUT_EXCLUSIVE;
    assert!(caps.has(Capabilities::READ));
    assert!(caps.has(Capabilities::WRITE));
    assert!(caps.has(Capabilities::EXCLUSIVE));
    assert!(caps.has(Capabilities::DELEGATE));
    assert!(caps.has(Capabilities::DROP));
    assert!(!caps.has(Capabilities::ALIAS));
}

#[test]
fn test_capabilities_attenuate() {
    let caps = Capabilities::FULL;
    let attenuated = caps.attenuate(Capabilities::WRITE | Capabilities::DROP);
    assert!(attenuated.has(Capabilities::READ));
    assert!(!attenuated.has(Capabilities::WRITE));
    assert!(!attenuated.has(Capabilities::DROP));
    assert!(attenuated.has(Capabilities::ADD));
}

#[test]
fn test_capabilities_intersect() {
    let a = Capabilities::new(Capabilities::READ | Capabilities::WRITE | Capabilities::ALIAS);
    let b = Capabilities::new(Capabilities::READ | Capabilities::EXCLUSIVE | Capabilities::ALIAS);
    let intersection = a.intersect(&b);
    assert!(intersection.has(Capabilities::READ));
    assert!(intersection.has(Capabilities::ALIAS));
    assert!(!intersection.has(Capabilities::WRITE));
    assert!(!intersection.has(Capabilities::EXCLUSIVE));
}

#[test]
fn test_capabilities_new() {
    let caps = Capabilities::new(Capabilities::READ | Capabilities::WRITE);
    assert!(caps.has(Capabilities::READ));
    assert!(caps.has(Capabilities::WRITE));
    assert!(!caps.has(Capabilities::ADD));
}

#[test]
fn test_capabilities_default() {
    let caps = Capabilities::default();
    assert!(!caps.has(Capabilities::READ));
    assert!(!caps.has(Capabilities::WRITE));
    assert_eq!(caps.0, 0);
}

// ============================================================================
// ThinRef Tests
// ============================================================================

#[test]
fn test_thin_ref_null() {
    let r = ThinRef::null();
    assert!(r.is_null());
    assert_eq!(r.generation, 0);
    assert_eq!(r.epoch(), 0);
    assert_eq!(r.capabilities(), Capabilities::default());
}

#[test]
fn test_thin_ref_creation() {
    let mut data: u64 = 42;
    let ptr = &mut data as *mut u64 as *mut u8;
    let caps = Capabilities::new(Capabilities::READ | Capabilities::WRITE);
    let r = ThinRef::new(ptr, 7, 3, caps);

    assert!(!r.is_null());
    assert_eq!(r.ptr, ptr);
    assert_eq!(r.generation, 7);
    assert_eq!(r.epoch(), 3);
    assert_eq!(r.capabilities(), caps);
}

#[test]
fn test_thin_ref_epoch_packing() {
    let r = ThinRef::new(std::ptr::null_mut(), 0, 0xFFFF, Capabilities::new(0));
    assert_eq!(r.epoch(), 0xFFFF);
    assert_eq!(r.capabilities(), Capabilities::new(0));
}

#[test]
fn test_thin_ref_caps_packing() {
    let r = ThinRef::new(std::ptr::null_mut(), 0, 0, Capabilities::new(0xFFFF));
    assert_eq!(r.epoch(), 0);
    assert_eq!(r.capabilities(), Capabilities::new(0xFFFF));
}

#[test]
fn test_thin_ref_epoch_and_caps_together() {
    let r = ThinRef::new(std::ptr::null_mut(), 0, 0x1234, Capabilities::new(0x5678));
    assert_eq!(r.epoch(), 0x1234);
    assert_eq!(r.capabilities(), Capabilities::new(0x5678));
}

#[test]
fn test_thin_ref_attenuate() {
    let caps = Capabilities::FULL;
    let r = ThinRef::new(std::ptr::null_mut(), 5, 10, caps);
    let attenuated = r.attenuate(Capabilities::WRITE);

    assert_eq!(attenuated.generation, 5);
    assert_eq!(attenuated.epoch(), 10);
    assert!(attenuated.capabilities().has(Capabilities::READ));
    assert!(!attenuated.capabilities().has(Capabilities::WRITE));
}

#[test]
fn test_thin_ref_size() {
    assert_eq!(std::mem::size_of::<ThinRef>(), 16);
}

#[test]
fn test_thin_ref_default() {
    let r = ThinRef::default();
    assert!(r.is_null());
}

#[test]
fn test_thin_ref_equality() {
    let r1 = ThinRef::new(std::ptr::null_mut(), 1, 2, Capabilities::new(3));
    let r2 = ThinRef::new(std::ptr::null_mut(), 1, 2, Capabilities::new(3));
    assert_eq!(r1, r2);
}

// ============================================================================
// FatRef Tests
// ============================================================================

#[test]
fn test_fat_ref_creation() {
    let caps = Capabilities::FULL;
    let r = FatRef::new(std::ptr::null_mut(), 7, 3, caps, 100);

    assert!(r.thin.is_null());
    assert_eq!(r.thin.generation, 7);
    assert_eq!(r.thin.epoch(), 3);
    assert_eq!(r.thin.capabilities(), caps);
    assert_eq!(r.metadata, 100);
    assert_eq!(r.offset, 0);
    assert_eq!(r.reserved, 0);
}

#[test]
fn test_fat_ref_size() {
    assert_eq!(std::mem::size_of::<FatRef>(), 32);
}

#[test]
fn test_fat_ref_with_slice_metadata() {
    let caps = Capabilities::READ_ONLY;
    let slice_len = 42u64;
    let r = FatRef::new(std::ptr::null_mut(), 1, 1, caps, slice_len);
    assert_eq!(r.metadata, 42);
}

#[test]
fn test_fat_ref_inherits_thin_ref() {
    let caps = Capabilities::FULL;
    let r = FatRef::new(std::ptr::null_mut(), 5, 10, caps, 0);

    // FatRef's thin portion should have correct values
    assert_eq!(r.thin.generation, 5);
    assert_eq!(r.thin.epoch(), 10);
    assert_eq!(r.thin.capabilities(), caps);
}

// ============================================================================
// Generation Counter Tests
// ============================================================================

#[test]
fn test_generation_counter_increment() {
    let r1 = ThinRef::new(std::ptr::null_mut(), 0, 1, Capabilities::FULL);
    let r2 = ThinRef::new(std::ptr::null_mut(), 1, 1, Capabilities::FULL);
    // Different generations mean the reference is stale
    assert_ne!(r1.generation, r2.generation);
}

#[test]
fn test_generation_wrap_around() {
    let r = ThinRef::new(std::ptr::null_mut(), u32::MAX, 0, Capabilities::FULL);
    assert_eq!(r.generation, u32::MAX);
}

// ============================================================================
// Epoch Capability Tests
// ============================================================================

#[test]
fn test_epoch_zero_is_valid() {
    let r = ThinRef::new(std::ptr::null_mut(), 0, 0, Capabilities::FULL);
    assert_eq!(r.epoch(), 0);
}

#[test]
fn test_epoch_max_value() {
    let r = ThinRef::new(std::ptr::null_mut(), 0, u16::MAX, Capabilities::FULL);
    assert_eq!(r.epoch(), u16::MAX);
}
