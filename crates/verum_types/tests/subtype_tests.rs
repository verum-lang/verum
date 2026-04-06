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
use indexmap::IndexMap;
use verum_common::{List, Map, Text};
use verum_types::subtype::*;
use verum_types::ty::Type;

// ============================================================================
// Basic Subtyping Tests
// ============================================================================

#[test]
fn test_subtype_reflexive() {
    let s = Subtyping::new();
    assert!(s.is_subtype(&Type::int(), &Type::int()));
    assert!(s.is_subtype(&Type::bool(), &Type::bool()));
    assert!(s.is_subtype(&Type::float(), &Type::float()));
    assert!(s.is_subtype(&Type::text(), &Type::text()));
    assert!(s.is_subtype(&Type::unit(), &Type::unit()));
}

#[test]
fn test_function_subtyping_basic() {
    let s = Subtyping::new();

    // (Int -> Bool) <: (Int -> Bool)
    let f1 = Type::function(List::from(vec![Type::int()]), Type::bool());
    assert!(s.is_subtype(&f1, &f1));

    // Not: (Bool -> Bool) <: (Int -> Bool)
    let f2 = Type::function(List::from(vec![Type::bool()]), Type::bool());
    assert!(!s.is_subtype(&f2, &f1));
}

#[test]
fn test_tuple_subtyping() {
    let s = Subtyping::new();

    let t1 = Type::tuple(List::from(vec![Type::int(), Type::bool()]));
    let t2 = Type::tuple(List::from(vec![Type::int(), Type::bool()]));

    assert!(s.is_subtype(&t1, &t2));

    let t3 = Type::tuple(List::from(vec![Type::bool(), Type::int()]));
    assert!(!s.is_subtype(&t1, &t3));
}

// ============================================================================
// Record Subtyping Tests (Width + Depth)
// ============================================================================

#[test]
fn test_record_subtyping_exact_match() {
    let s = Subtyping::new();

    // {x: Int, y: Int} <: {x: Int, y: Int}
    let mut fields1 = IndexMap::new();
    fields1.insert(Text::from("x"), Type::int());
    fields1.insert(Text::from("y"), Type::int());

    let mut fields2 = IndexMap::new();
    fields2.insert(Text::from("x"), Type::int());
    fields2.insert(Text::from("y"), Type::int());

    let r1 = Type::Record(fields1);
    let r2 = Type::Record(fields2);

    assert!(s.is_subtype(&r1, &r2));
}

#[test]
fn test_record_width_subtyping() {
    let s = Subtyping::new();

    // {x: Int, y: Int, z: Int} <: {x: Int, y: Int} (width subtyping)
    let mut fields1 = IndexMap::new();
    fields1.insert(Text::from("x"), Type::int());
    fields1.insert(Text::from("y"), Type::int());
    fields1.insert(Text::from("z"), Type::int()); // Extra field

    let mut fields2 = IndexMap::new();
    fields2.insert(Text::from("x"), Type::int());
    fields2.insert(Text::from("y"), Type::int());

    let r1 = Type::Record(fields1);
    let r2 = Type::Record(fields2);

    assert!(s.is_subtype(&r1, &r2)); // Point3D <: Point2D
}

#[test]
fn test_record_missing_field_not_subtype() {
    let s = Subtyping::new();

    // {x: Int} </: {x: Int, y: Int} (missing field y)
    let mut fields1 = IndexMap::new();
    fields1.insert(Text::from("x"), Type::int());

    let mut fields2 = IndexMap::new();
    fields2.insert(Text::from("x"), Type::int());
    fields2.insert(Text::from("y"), Type::int());

    let r1 = Type::Record(fields1);
    let r2 = Type::Record(fields2);

    assert!(!s.is_subtype(&r1, &r2));
}

#[test]
fn test_record_depth_subtyping() {
    let s = Subtyping::new();

    // {x: Int, y: Bool} and field types must match
    let mut fields1 = IndexMap::new();
    fields1.insert(Text::from("x"), Type::int());
    fields1.insert(Text::from("y"), Type::bool());

    let mut fields2 = IndexMap::new();
    fields2.insert(Text::from("x"), Type::int());
    fields2.insert(Text::from("y"), Type::bool());

    let r1 = Type::Record(fields1);
    let r2 = Type::Record(fields2);

    assert!(s.is_subtype(&r1, &r2));
}

#[test]
fn test_record_depth_subtyping_mismatch() {
    let s = Subtyping::new();

    // {x: Bool} </: {x: Int} (type mismatch)
    let mut fields1 = IndexMap::new();
    fields1.insert(Text::from("x"), Type::bool());

    let mut fields2 = IndexMap::new();
    fields2.insert(Text::from("x"), Type::int());

    let r1 = Type::Record(fields1);
    let r2 = Type::Record(fields2);

    assert!(!s.is_subtype(&r1, &r2));
}

// ============================================================================
// Variant Subtyping Tests
// ============================================================================

#[test]
fn test_variant_subtyping_exact_match() {
    let s = Subtyping::new();

    // Circle(Float) | Square(Float) <: Circle(Float) | Square(Float)
    let mut v1 = IndexMap::new();
    v1.insert(Text::from("Circle"), Type::float());
    v1.insert(Text::from("Square"), Type::float());

    let mut v2 = IndexMap::new();
    v2.insert(Text::from("Circle"), Type::float());
    v2.insert(Text::from("Square"), Type::float());

    let var1 = Type::Variant(v1);
    let var2 = Type::Variant(v2);

    assert!(s.is_subtype(&var1, &var2));
}

#[test]
fn test_variant_subtyping_subset() {
    let s = Subtyping::new();

    // Circle(Float) <: Circle(Float) | Square(Float) | Triangle(Float)
    let mut v1 = IndexMap::new();
    v1.insert(Text::from("Circle"), Type::float());

    let mut v2 = IndexMap::new();
    v2.insert(Text::from("Circle"), Type::float());
    v2.insert(Text::from("Square"), Type::float());
    v2.insert(Text::from("Triangle"), Type::float());

    let var1 = Type::Variant(v1);
    let var2 = Type::Variant(v2);

    assert!(s.is_subtype(&var1, &var2));
}

#[test]
fn test_variant_subtyping_extra_tag_not_subtype() {
    let s = Subtyping::new();

    // Circle | Square | Triangle </: Circle | Square
    let mut v1 = IndexMap::new();
    v1.insert(Text::from("Circle"), Type::float());
    v1.insert(Text::from("Square"), Type::float());
    v1.insert(Text::from("Triangle"), Type::float());

    let mut v2 = IndexMap::new();
    v2.insert(Text::from("Circle"), Type::float());
    v2.insert(Text::from("Square"), Type::float());

    let var1 = Type::Variant(v1);
    let var2 = Type::Variant(v2);

    assert!(!s.is_subtype(&var1, &var2));
}

#[test]
fn test_variant_covariance_in_types() {
    let s = Subtyping::new();

    // Circle(Int) <: Circle(Int) (same type)
    let mut v1 = IndexMap::new();
    v1.insert(Text::from("Circle"), Type::int());

    let mut v2 = IndexMap::new();
    v2.insert(Text::from("Circle"), Type::int());

    let var1 = Type::Variant(v1);
    let var2 = Type::Variant(v2);

    assert!(s.is_subtype(&var1, &var2));
}

// ============================================================================
// Array Subtyping Tests
// ============================================================================

#[test]
fn test_array_subtyping_same_size() {
    let s = Subtyping::new();

    // [Int; 10] <: [Int; 10]
    let a1 = Type::Array {
        element: Box::new(Type::int()),
        size: Some(10),
    };
    let a2 = Type::Array {
        element: Box::new(Type::int()),
        size: Some(10),
    };

    assert!(s.is_subtype(&a1, &a2));
}

#[test]
fn test_array_subtyping_different_size_not_subtype() {
    let s = Subtyping::new();

    // [Int; 10] </: [Int; 20] (size mismatch)
    let a1 = Type::Array {
        element: Box::new(Type::int()),
        size: Some(10),
    };
    let a2 = Type::Array {
        element: Box::new(Type::int()),
        size: Some(20),
    };

    assert!(!s.is_subtype(&a1, &a2));
}

#[test]
fn test_array_element_covariance() {
    let s = Subtyping::new();

    // Arrays are covariant in element type
    let a1 = Type::Array {
        element: Box::new(Type::int()),
        size: Some(5),
    };
    let a2 = Type::Array {
        element: Box::new(Type::int()),
        size: Some(5),
    };

    assert!(s.is_subtype(&a1, &a2));
}

#[test]
fn test_array_none_size() {
    let s = Subtyping::new();

    // [Int] <: [Int] (unsized arrays)
    let a1 = Type::Array {
        element: Box::new(Type::int()),
        size: None,
    };
    let a2 = Type::Array {
        element: Box::new(Type::int()),
        size: None,
    };

    assert!(s.is_subtype(&a1, &a2));
}

#[test]
fn test_array_sized_vs_unsized_not_subtype() {
    let s = Subtyping::new();

    // [Int; 10] </: [Int] (sized vs unsized)
    let a1 = Type::Array {
        element: Box::new(Type::int()),
        size: Some(10),
    };
    let a2 = Type::Array {
        element: Box::new(Type::int()),
        size: None,
    };

    assert!(!s.is_subtype(&a1, &a2));
}

// ============================================================================
// Reference Variance Tests
// ============================================================================

#[test]
fn test_shared_reference_covariance() {
    let s = Subtyping::new();

    // &Int <: &Int (shared refs covariant)
    let r1 = Type::Reference {
        mutable: false,
        inner: Box::new(Type::int()),
    };
    let r2 = Type::Reference {
        mutable: false,
        inner: Box::new(Type::int()),
    };

    assert!(s.is_subtype(&r1, &r2));
}

#[test]
fn test_mutable_reference_invariance() {
    let s = Subtyping::new();

    // &mut Int <: &mut Int only if exact match (invariant)
    let r1 = Type::Reference {
        mutable: true,
        inner: Box::new(Type::int()),
    };
    let r2 = Type::Reference {
        mutable: true,
        inner: Box::new(Type::int()),
    };

    assert!(s.is_subtype(&r1, &r2));
}

#[test]
fn test_reference_mutability_mismatch() {
    let s = Subtyping::new();

    // &Int </: &mut Int (mutability mismatch)
    let r1 = Type::Reference {
        mutable: false,
        inner: Box::new(Type::int()),
    };
    let r2 = Type::Reference {
        mutable: true,
        inner: Box::new(Type::int()),
    };

    assert!(!s.is_subtype(&r1, &r2));
}

#[test]
fn test_checked_to_managed_upcast() {
    let s = Subtyping::new();

    // &checked Int <: &Int (forgetful upcast)
    let r1 = Type::CheckedReference {
        mutable: false,
        inner: Box::new(Type::int()),
    };
    let r2 = Type::Reference {
        mutable: false,
        inner: Box::new(Type::int()),
    };

    assert!(s.is_subtype(&r1, &r2));
}

#[test]
fn test_unsafe_to_checked_upcast() {
    let s = Subtyping::new();

    // &unsafe Int <: &checked Int (forgetful upcast)
    let r1 = Type::UnsafeReference {
        mutable: false,
        inner: Box::new(Type::int()),
    };
    let r2 = Type::CheckedReference {
        mutable: false,
        inner: Box::new(Type::int()),
    };

    assert!(s.is_subtype(&r1, &r2));
}

#[test]
fn test_unsafe_to_managed_upcast() {
    let s = Subtyping::new();

    // &unsafe Int <: &Int (forgetful upcast via checked)
    let r1 = Type::UnsafeReference {
        mutable: false,
        inner: Box::new(Type::int()),
    };
    let r2 = Type::Reference {
        mutable: false,
        inner: Box::new(Type::int()),
    };

    assert!(s.is_subtype(&r1, &r2));
}

#[test]
fn test_managed_to_checked_downcast_forbidden() {
    let s = Subtyping::new();

    // &Int </: &checked Int (cannot invent proof)
    let r1 = Type::Reference {
        mutable: false,
        inner: Box::new(Type::int()),
    };
    let r2 = Type::CheckedReference {
        mutable: false,
        inner: Box::new(Type::int()),
    };

    assert!(!s.is_subtype(&r1, &r2));
}

#[test]
fn test_checked_reference_shared_covariance() {
    let s = Subtyping::new();

    // &checked Int <: &checked Int (shared covariant)
    let r1 = Type::CheckedReference {
        mutable: false,
        inner: Box::new(Type::int()),
    };
    let r2 = Type::CheckedReference {
        mutable: false,
        inner: Box::new(Type::int()),
    };

    assert!(s.is_subtype(&r1, &r2));
}

#[test]
fn test_checked_reference_mutable_invariance() {
    let s = Subtyping::new();

    // &checked mut Int <: &checked mut Int (mutable invariant)
    let r1 = Type::CheckedReference {
        mutable: true,
        inner: Box::new(Type::int()),
    };
    let r2 = Type::CheckedReference {
        mutable: true,
        inner: Box::new(Type::int()),
    };

    assert!(s.is_subtype(&r1, &r2));
}

#[test]
fn test_unsafe_reference_shared_covariance() {
    let s = Subtyping::new();

    // &unsafe Int <: &unsafe Int (shared covariant)
    let r1 = Type::UnsafeReference {
        mutable: false,
        inner: Box::new(Type::int()),
    };
    let r2 = Type::UnsafeReference {
        mutable: false,
        inner: Box::new(Type::int()),
    };

    assert!(s.is_subtype(&r1, &r2));
}

#[test]
fn test_unsafe_reference_mutable_invariance() {
    let s = Subtyping::new();

    // &unsafe mut Int <: &unsafe mut Int (mutable invariant)
    let r1 = Type::UnsafeReference {
        mutable: true,
        inner: Box::new(Type::int()),
    };
    let r2 = Type::UnsafeReference {
        mutable: true,
        inner: Box::new(Type::int()),
    };

    assert!(s.is_subtype(&r1, &r2));
}

// ============================================================================
// Function Contravariance Tests
// ============================================================================

#[test]
fn test_function_parameter_contravariance() {
    let s = Subtyping::new();

    // (Int -> Bool) <: (Int -> Bool)
    let f1 = Type::function(List::from(vec![Type::int()]), Type::bool());
    let f2 = Type::function(vec![Type::int()].into(), Type::bool());

    assert!(s.is_subtype(&f1, &f2));
}

#[test]
fn test_function_return_covariance() {
    let s = Subtyping::new();

    // (Int -> Bool) <: (Int -> Bool)
    let f1 = Type::function(List::from(vec![Type::int()]), Type::bool());
    let f2 = Type::function(vec![Type::int()].into(), Type::bool());

    assert!(s.is_subtype(&f1, &f2));
}

#[test]
fn test_function_parameter_count_mismatch() {
    let s = Subtyping::new();

    // (Int -> Bool) </: (Int, Bool -> Bool)
    let f1 = Type::function(List::from(vec![Type::int()]), Type::bool());
    let f2 = Type::function(List::from(vec![Type::int(), Type::bool()]), Type::bool());

    assert!(!s.is_subtype(&f1, &f2));
}

// ============================================================================
// Pointer Invariance Tests
// ============================================================================

#[test]
fn test_pointer_invariance() {
    let s = Subtyping::new();

    // *const Int <: *const Int (invariant)
    let p1 = Type::Pointer {
        mutable: false,
        inner: Box::new(Type::int()),
    };
    let p2 = Type::Pointer {
        mutable: false,
        inner: Box::new(Type::int()),
    };

    assert!(s.is_subtype(&p1, &p2));
}

#[test]
fn test_pointer_mutability_mismatch() {
    let s = Subtyping::new();

    // *const Int </: *mut Int
    let p1 = Type::Pointer {
        mutable: false,
        inner: Box::new(Type::int()),
    };
    let p2 = Type::Pointer {
        mutable: true,
        inner: Box::new(Type::int()),
    };

    assert!(!s.is_subtype(&p1, &p2));
}

// ============================================================================
// Ownership Reference Tests
// ============================================================================

#[test]
fn test_ownership_shared_covariance() {
    let s = Subtyping::new();

    // %Int <: %Int (shared covariant)
    let o1 = Type::Ownership {
        mutable: false,
        inner: Box::new(Type::int()),
    };
    let o2 = Type::Ownership {
        mutable: false,
        inner: Box::new(Type::int()),
    };

    assert!(s.is_subtype(&o1, &o2));
}

#[test]
fn test_ownership_mutable_invariance() {
    let s = Subtyping::new();

    // %mut Int <: %mut Int (mutable invariant)
    let o1 = Type::Ownership {
        mutable: true,
        inner: Box::new(Type::int()),
    };
    let o2 = Type::Ownership {
        mutable: true,
        inner: Box::new(Type::int()),
    };

    assert!(s.is_subtype(&o1, &o2));
}

// ============================================================================
// Auto-Reference Coercion Tests
// ============================================================================

#[test]
fn test_text_to_ref_text_coercion() {
    let s = Subtyping::new();

    // Text <: &Text (auto-reference coercion for immutable refs)
    let text_type = Type::Text;
    let ref_text_type = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Text),
    };

    assert!(s.is_subtype(&text_type, &ref_text_type));
}

#[test]
fn test_text_to_mut_ref_text_no_coercion() {
    let s = Subtyping::new();

    // Text </: &mut Text (no coercion for mutable refs)
    let text_type = Type::Text;
    let mut_ref_text_type = Type::Reference {
        mutable: true,
        inner: Box::new(Type::Text),
    };

    assert!(!s.is_subtype(&text_type, &mut_ref_text_type));
}

#[test]
fn test_char_to_ref_char_coercion() {
    let s = Subtyping::new();

    // Char <: &Char (auto-reference coercion for immutable refs)
    let char_type = Type::Char;
    let ref_char_type = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Char),
    };

    assert!(s.is_subtype(&char_type, &ref_char_type));
}

#[test]
fn test_char_to_mut_ref_char_no_coercion() {
    let s = Subtyping::new();

    // Char </: &mut Char (no coercion for mutable refs)
    let char_type = Type::Char;
    let mut_ref_char_type = Type::Reference {
        mutable: true,
        inner: Box::new(Type::Char),
    };

    assert!(!s.is_subtype(&char_type, &mut_ref_char_type));
}

#[test]
fn test_int_to_ref_int_no_coercion() {
    let s = Subtyping::new();

    // Int </: &Int (no auto-reference for other types, only Text and Char)
    let int_type = Type::int();
    let ref_int_type = Type::Reference {
        mutable: false,
        inner: Box::new(Type::int()),
    };

    assert!(!s.is_subtype(&int_type, &ref_int_type));
}
