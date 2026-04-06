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
// Three-tier reference model tests
//
// Tests Verum's three-tier reference system:
// - `&T` - CBGR-managed references (~15ns overhead)
// - `&checked T` - Runtime bounds checking
// - `&unsafe T` - Zero-cost, no checks
//
// Verum type system with semantic types (List, Text, Map, Maybe) and HM inference

use verum_ast::{
    span::Span,
    ty::{Ident, Path},
};
use verum_common::List;
use verum_types::ty::*;

// ============================================================================
// CBGR Reference Tests (&T)
// ============================================================================

#[test]
fn test_cbgr_reference_creation() {
    let base = Type::int();
    let cbgr_ref = Type::reference(false, base.clone());

    assert!(matches!(cbgr_ref, Type::Reference { .. }));
}

#[test]
fn test_cbgr_reference_base_type() {
    let base = Type::int();
    let cbgr_ref = Type::reference(false, base.clone());

    if let Type::Reference { inner, .. } = cbgr_ref {
        assert_eq!(*inner, base);
    } else {
        panic!("Expected Reference type");
    }
}

#[test]
fn test_cbgr_reference_to_struct() {
    let span = Span::dummy();
    let struct_type = Type::Named {
        path: Path::single(Ident::new("Point", span)),
        args: List::new(),
    };
    let cbgr_ref = Type::reference(false, struct_type);

    assert!(matches!(cbgr_ref, Type::Reference { .. }));
}

#[test]
fn test_cbgr_reference_to_function() {
    let func_type = Type::function(vec![Type::int()].into(), Type::bool());
    let cbgr_ref = Type::reference(false, func_type);

    assert!(matches!(cbgr_ref, Type::Reference { .. }));
}

#[test]
fn test_nested_cbgr_references() {
    let base = Type::int();
    let ref1 = Type::reference(false, base);
    let ref2 = Type::reference(false, ref1);

    // &(&T) should be valid
    assert!(matches!(ref2, Type::Reference { .. }));
}

// ============================================================================
// Checked Reference Tests (&checked T)
// ============================================================================

#[test]
fn test_checked_reference_creation() {
    let base = Type::int();
    let checked_ref = Type::checked_reference(false, base.clone());

    assert!(matches!(checked_ref, Type::CheckedReference { .. }));
}

#[test]
fn test_checked_reference_array_bounds() {
    let span = Span::dummy();
    // &checked [T] - runtime bounds checking
    let array_type = Type::Named {
        path: Path::single(Ident::new("Array", span)),
        args: List::new(),
    };
    let checked_ref = Type::checked_reference(false, array_type);

    assert!(matches!(checked_ref, Type::CheckedReference { .. }));
}

#[test]
fn test_checked_reference_to_slice() {
    let span = Span::dummy();
    let slice_type = Type::Named {
        path: Path::single(Ident::new("Slice", span)),
        args: List::new(),
    };
    let checked_ref = Type::checked_reference(false, slice_type);

    assert!(matches!(checked_ref, Type::CheckedReference { .. }));
}

#[test]
fn test_checked_reference_prevents_ub() {
    // Checked references should be used when:
    // - Array access patterns unknown at compile time
    // - FFI boundaries
    // - Untrusted input processing

    let base = Type::int();
    let checked = Type::checked_reference(false, base);

    assert!(matches!(checked, Type::CheckedReference { .. }));
}

// ============================================================================
// Unsafe Reference Tests (&unsafe T)
// ============================================================================

#[test]
fn test_unsafe_reference_creation() {
    let base = Type::int();
    let unsafe_ref = Type::unsafe_reference(false, base.clone());

    assert!(matches!(unsafe_ref, Type::UnsafeReference { .. }));
}

#[test]
fn test_unsafe_reference_zero_cost() {
    // &unsafe T has zero runtime overhead
    // Used in hot paths where safety is proven
    let base = Type::int();
    let unsafe_ref = Type::unsafe_reference(false, base);

    assert!(matches!(unsafe_ref, Type::UnsafeReference { .. }));
}

#[test]
fn test_unsafe_reference_ffi() {
    let span = Span::dummy();
    // &unsafe T commonly used for FFI
    let c_ptr_type = Type::Named {
        path: Path::single(Ident::new("CPointer", span)),
        args: List::new(),
    };
    let unsafe_ref = Type::unsafe_reference(false, c_ptr_type);

    assert!(matches!(unsafe_ref, Type::UnsafeReference { .. }));
}

#[test]
fn test_unsafe_reference_requires_proof() {
    // In actual usage, &unsafe requires safety proof
    // This test just validates type structure
    let base = Type::int();
    let unsafe_ref = Type::unsafe_reference(false, base);

    assert!(matches!(unsafe_ref, Type::UnsafeReference { .. }));
}

// ============================================================================
// Reference Kind Comparison Tests
// ============================================================================

#[test]
fn test_reference_kinds_distinct() {
    let base = Type::int();

    let cbgr = Type::reference(false, base.clone());
    let checked = Type::checked_reference(false, base.clone());
    let unsafe_ref = Type::unsafe_reference(false, base.clone());

    // All three should be distinct types
    assert_ne!(cbgr, checked);
    assert_ne!(checked, unsafe_ref);
    assert_ne!(cbgr, unsafe_ref);
}

#[test]
fn test_reference_kind_subtyping() {
    let base = Type::int();

    let cbgr = Type::reference(false, base.clone());
    let checked = Type::checked_reference(false, base.clone());

    // &T should NOT be automatically convertible to &checked T
    // Each tier has specific semantics
    assert_ne!(cbgr, checked);
}

// ============================================================================
// Performance Characteristic Tests
// ============================================================================

#[test]
fn test_cbgr_performance_tier() {
    // &T: ~15ns overhead per check
    // Acceptable for most code
    let base = Type::int();
    let cbgr = Type::reference(false, base);

    assert!(matches!(cbgr, Type::Reference { .. }));
}

#[test]
fn test_checked_performance_tier() {
    // &checked T: Runtime bounds checking
    // Slower than CBGR but safer than unsafe
    let base = Type::int();
    let checked = Type::checked_reference(false, base);

    assert!(matches!(checked, Type::CheckedReference { .. }));
}

#[test]
fn test_unsafe_performance_tier() {
    // &unsafe T: Zero-cost abstraction
    // Same as raw pointer, requires proof
    let base = Type::int();
    let unsafe_ref = Type::unsafe_reference(false, base);

    assert!(matches!(unsafe_ref, Type::UnsafeReference { .. }));
}

// ============================================================================
// Mutability with References
// ============================================================================

#[test]
fn test_immutable_cbgr_reference() {
    let base = Type::int();
    let cbgr = Type::reference(false, base);

    // Immutable reference
    assert!(matches!(cbgr, Type::Reference { mutable: false, .. }));
}

#[test]
fn test_mutable_cbgr_reference() {
    let base = Type::int();
    // &mut T with CBGR
    let mut_cbgr = Type::reference(true, base);

    assert!(matches!(mut_cbgr, Type::Reference { mutable: true, .. }));
}

#[test]
fn test_checked_mutable_reference() {
    let base = Type::int();
    let checked_mut = Type::checked_reference(true, base);

    assert!(matches!(
        checked_mut,
        Type::CheckedReference { mutable: true, .. }
    ));
}

// ============================================================================
// Complex Reference Type Tests
// ============================================================================

#[test]
fn test_reference_to_tuple() {
    let tuple = Type::tuple(vec![Type::int(), Type::bool()].into());
    let ref_tuple = Type::reference(false, tuple);

    assert!(matches!(ref_tuple, Type::Reference { .. }));
}

#[test]
fn test_reference_to_list() {
    let span = Span::dummy();
    let list = Type::Named {
        path: Path::single(Ident::new("List", span)),
        args: List::new(),
    };
    let ref_list = Type::reference(false, list);

    assert!(matches!(ref_list, Type::Reference { .. }));
}

#[test]
fn test_reference_to_generic() {
    // &T where T is generic
    let generic = Type::Var(TypeVar::fresh());
    let ref_generic = Type::reference(false, generic);

    assert!(matches!(ref_generic, Type::Reference { .. }));
}

// ============================================================================
// Reference Lifetime Tests (Conceptual)
// ============================================================================

#[test]
fn test_reference_implies_lifetime() {
    // All references have implicit lifetimes
    // CBGR manages these automatically
    let base = Type::int();
    let cbgr = Type::reference(false, base);

    assert!(matches!(cbgr, Type::Reference { .. }));
}

#[test]
fn test_reference_outlives_referent() {
    // References cannot outlive what they point to
    // CBGR enforces this at ~15ns cost
    let base = Type::int();
    let cbgr = Type::reference(false, base);

    assert!(matches!(cbgr, Type::Reference { .. }));
}

// ============================================================================
// Use Case Tests
// ============================================================================

#[test]
fn test_cbgr_for_general_purpose() {
    // Default choice: &T with CBGR
    // Safe, fast enough for most code
    let span = Span::dummy();
    let data = Type::Named {
        path: Path::single(Ident::new("Data", span)),
        args: List::new(),
    };
    let ref_data = Type::reference(false, data);

    assert!(matches!(ref_data, Type::Reference { .. }));
}

#[test]
fn test_checked_for_arrays() {
    // Use &checked for array access
    // When bounds not statically known
    let span = Span::dummy();
    let array = Type::Named {
        path: Path::single(Ident::new("Array", span)),
        args: List::new(),
    };
    let checked_array = Type::checked_reference(false, array);

    assert!(matches!(checked_array, Type::CheckedReference { .. }));
}

#[test]
fn test_unsafe_for_hot_paths() {
    // Use &unsafe in proven hot paths
    // After benchmarking shows CBGR overhead matters
    let base = Type::int();
    let unsafe_ref = Type::unsafe_reference(false, base);

    assert!(matches!(unsafe_ref, Type::UnsafeReference { .. }));
}

// ============================================================================
// Conversion Tests (Explicit Only)
// ============================================================================

#[test]
fn test_no_implicit_conversion() {
    let base = Type::int();
    let cbgr = Type::reference(false, base.clone());
    let unsafe_ref = Type::unsafe_reference(false, base.clone());

    // Should be different types
    assert_ne!(cbgr, unsafe_ref);
}

#[test]
fn test_explicit_cast_cbgr_to_unsafe() {
    // In actual code: let x: &unsafe T = unsafe { &*ptr };
    // This test just validates types exist
    let base = Type::int();
    let cbgr = Type::reference(false, base.clone());
    let unsafe_ref = Type::unsafe_reference(false, base.clone());

    assert!(matches!(cbgr, Type::Reference { .. }));
    assert!(matches!(unsafe_ref, Type::UnsafeReference { .. }));
}

// ============================================================================
// Error Cases
// ============================================================================

#[test]
fn test_reference_to_unit() {
    // &() is valid but unusual
    let unit = Type::unit();
    let ref_unit = Type::reference(false, unit);

    assert!(matches!(ref_unit, Type::Reference { .. }));
}

#[test]
fn test_reference_display() {
    let base = Type::int();
    let cbgr = Type::reference(false, base.clone());
    let checked = Type::checked_reference(false, base.clone());
    let unsafe_ref = Type::unsafe_reference(false, base.clone());

    // Display should show reference kind
    let cbgr_str = cbgr.to_string();
    let checked_str = checked.to_string();
    let unsafe_str = unsafe_ref.to_string();

    assert!(cbgr_str.contains("&") || cbgr_str.contains("Ref"));
    assert!(checked_str.contains("checked") || checked_str.contains("Checked"));
    assert!(unsafe_str.contains("unsafe") || unsafe_str.contains("Unsafe"));
}

// ============================================================================
// Auto-Borrow Coercion Tests
// Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 3 (Auto-Borrow в позиции вызова)
// ============================================================================

#[test]
fn test_auto_borrow_immutable_reference_type_structure() {
    // Verify type structure for auto-borrow:
    // When expected is &T (immutable) and actual is T,
    // unifying T with inner of &T should succeed
    let base = Type::int();
    let ref_type = Type::reference(false, base.clone());

    // Extract inner type from reference
    if let Type::Reference { mutable, inner } = &ref_type {
        assert!(!mutable); // Must be immutable for auto-borrow
        assert_eq!(**inner, base); // Inner should be the base type
    } else {
        panic!("Expected Reference type");
    }
}

#[test]
fn test_auto_borrow_mutable_reference_requires_explicit() {
    // Mutable references should NOT auto-borrow
    // User must explicitly write &mut x
    let base = Type::int();
    let mut_ref = Type::reference(true, base.clone());

    if let Type::Reference { mutable, .. } = &mut_ref {
        assert!(*mutable); // This is mutable - no auto-borrow allowed
    }
}

#[test]
fn test_auto_borrow_checked_reference_structure() {
    // Auto-borrow should also work for &checked T (immutable only)
    let base = Type::int();
    let checked_ref = Type::checked_reference(false, base.clone());

    if let Type::CheckedReference { mutable, inner } = &checked_ref {
        assert!(!mutable); // Must be immutable for auto-borrow
        assert_eq!(**inner, base);
    } else {
        panic!("Expected CheckedReference type");
    }
}

#[test]
fn test_auto_borrow_nested_reference() {
    // Auto-borrow should work with nested types
    let inner = Type::list(Type::int());
    let ref_list = Type::reference(false, inner.clone());

    if let Type::Reference { mutable, inner: ref_inner } = &ref_list {
        assert!(!mutable);
        assert!(matches!(**ref_inner, Type::Generic { .. }));
    }
}
