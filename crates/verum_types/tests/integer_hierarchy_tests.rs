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
use verum_types::integer_hierarchy::*;

#[test]
fn test_bit_widths() {
    assert_eq!(IntegerKind::Int8.bit_width(), Some(8));
    assert_eq!(IntegerKind::Int32.bit_width(), Some(32));
    assert_eq!(IntegerKind::UInt64.bit_width(), Some(64));
    assert_eq!(IntegerKind::Int.bit_width(), None);
}

#[test]
fn test_signedness() {
    assert!(IntegerKind::Int32.is_signed());
    assert!(!IntegerKind::Int32.is_unsigned());
    assert!(IntegerKind::UInt32.is_unsigned());
    assert!(!IntegerKind::UInt32.is_signed());
    assert!(IntegerKind::Int.is_signed());
}

#[test]
fn test_min_max_values() {
    assert_eq!(IntegerKind::Int8.min_value(), -128);
    assert_eq!(IntegerKind::Int8.max_value(), 127);
    assert_eq!(IntegerKind::UInt8.min_value(), 0);
    assert_eq!(IntegerKind::UInt8.max_value(), 255);
}

#[test]
fn test_name_parsing_semantic() {
    // Test semantic names (primary)
    assert_eq!(IntegerKind::from_name("Int32"), Some(IntegerKind::Int32));
    assert_eq!(IntegerKind::from_name("UInt64"), Some(IntegerKind::UInt64));
    assert_eq!(IntegerKind::from_name("Int"), Some(IntegerKind::Int));
    assert_eq!(IntegerKind::from_name("invalid"), None);
}

#[test]
fn test_name_parsing_compat() {
    // Test compatibility aliases (FFI)
    assert_eq!(IntegerKind::from_name("i32"), Some(IntegerKind::Int32));
    assert_eq!(IntegerKind::from_name("u64"), Some(IntegerKind::UInt64));
    assert_eq!(IntegerKind::from_name("i8"), Some(IntegerKind::Int8));
    assert_eq!(IntegerKind::from_name("u8"), Some(IntegerKind::UInt8));
    assert_eq!(IntegerKind::from_name("isize"), Some(IntegerKind::ISize));
    assert_eq!(IntegerKind::from_name("usize"), Some(IntegerKind::USize));
    // Byte is an alias for UInt8
    assert_eq!(IntegerKind::from_name("Byte"), Some(IntegerKind::UInt8));
}

#[test]
fn test_semantic_names() {
    assert_eq!(IntegerKind::Int8.semantic_name(), "Int8");
    assert_eq!(IntegerKind::Int32.semantic_name(), "Int32");
    assert_eq!(IntegerKind::UInt64.semantic_name(), "UInt64");
    assert_eq!(IntegerKind::ISize.semantic_name(), "ISize");
}

#[test]
fn test_compat_names() {
    assert_eq!(IntegerKind::Int8.compat_name(), "i8");
    assert_eq!(IntegerKind::Int32.compat_name(), "i32");
    assert_eq!(IntegerKind::UInt64.compat_name(), "u64");
    assert_eq!(IntegerKind::ISize.compat_name(), "isize");
}

#[test]
fn test_hierarchy_creation() {
    let hierarchy = IntegerHierarchy::new();
    assert!(hierarchy.get_type(IntegerKind::Int32).is_some());
    assert!(hierarchy.get_type(IntegerKind::UInt8).is_some());
}

#[test]
fn test_literal_fits() {
    let hierarchy = IntegerHierarchy::new();
    assert!(hierarchy.check_literal_fits(127, IntegerKind::Int8));
    assert!(!hierarchy.check_literal_fits(128, IntegerKind::Int8));
    assert!(hierarchy.check_literal_fits(255, IntegerKind::UInt8));
    assert!(!hierarchy.check_literal_fits(256, IntegerKind::UInt8));
}

#[test]
fn test_subtyping() {
    let hierarchy = IntegerHierarchy::new();

    // Int is top type
    assert!(hierarchy.is_subtype(IntegerKind::Int32, IntegerKind::Int));
    assert!(hierarchy.is_subtype(IntegerKind::UInt8, IntegerKind::Int));

    // Same type
    assert!(hierarchy.is_subtype(IntegerKind::Int32, IntegerKind::Int32));

    // Range subsumption
    assert!(hierarchy.is_subtype(IntegerKind::Int8, IntegerKind::Int16));
    assert!(hierarchy.is_subtype(IntegerKind::UInt8, IntegerKind::UInt16));

    // NOT subtype
    assert!(!hierarchy.is_subtype(IntegerKind::Int16, IntegerKind::Int8));
    assert!(!hierarchy.is_subtype(IntegerKind::UInt8, IntegerKind::Int8));
}

#[test]
fn test_suffix_inference() {
    let hierarchy = IntegerHierarchy::new();
    // Compat suffixes still work for FFI
    assert_eq!(hierarchy.infer_from_suffix("i32"), Some(IntegerKind::Int32));
    assert_eq!(hierarchy.infer_from_suffix("u64"), Some(IntegerKind::UInt64));
    // Semantic suffixes also work
    assert_eq!(hierarchy.infer_from_suffix("Int32"), Some(IntegerKind::Int32));
    assert_eq!(hierarchy.infer_from_suffix("UInt64"), Some(IntegerKind::UInt64));
    assert_eq!(hierarchy.infer_from_suffix("invalid"), None);
}

#[test]
fn test_defaults() {
    assert_eq!(IntegerHierarchy::default_signed(), IntegerKind::Int32);
    assert_eq!(IntegerHierarchy::default_unsigned(), IntegerKind::UInt32);
}

#[test]
fn test_max_value_u128() {
    // Test the full precision max value for unsigned types
    assert_eq!(IntegerKind::UInt64.max_value_u128(), u64::MAX as u128);
    assert_eq!(IntegerKind::UInt128.max_value_u128(), u128::MAX);
    assert_eq!(IntegerKind::UInt8.max_value_u128(), 255);
}
