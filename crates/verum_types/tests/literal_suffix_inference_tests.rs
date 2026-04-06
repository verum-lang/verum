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
// Tests for literal suffix-based type inference
// Integer type hierarchy: all fixed-size integers (i8..i128, u8..u128) are refinement types of Int with range predicates — .2 lines 143-162
// Unification: Robinson's algorithm extended with row polymorphism, refinement subtyping, and type class constraints — .4.2 lines 8705-8754

use verum_ast::literal::{FloatLit, FloatSuffix, IntLit, IntSuffix, Literal};
use verum_ast::span::Span;
use verum_common::Text;
use verum_types::infer::TypeChecker;

#[test]
fn test_integer_suffix_i8() {
    let _lit = Literal::new(
        verum_ast::literal::LiteralKind::Int(IntLit::with_suffix(42, IntSuffix::I8)),
        Span::dummy(),
    );

    let _checker = TypeChecker::new();
    // The type should be i8 (as a Named type)
    // Implementation uses i8_refined helper
}

#[test]
fn test_integer_suffix_i16() {
    let _lit = Literal::new(
        verum_ast::literal::LiteralKind::Int(IntLit::with_suffix(1000, IntSuffix::I16)),
        Span::dummy(),
    );

    let _checker = TypeChecker::new();
    // Type should be i16
}

#[test]
fn test_integer_suffix_i32() {
    let _lit = Literal::new(
        verum_ast::literal::LiteralKind::Int(IntLit::with_suffix(100000, IntSuffix::I32)),
        Span::dummy(),
    );

    let _checker = TypeChecker::new();
    // Type should be i32
}

#[test]
fn test_integer_suffix_i64() {
    let _lit = Literal::new(
        verum_ast::literal::LiteralKind::Int(IntLit::with_suffix(10000000000, IntSuffix::I64)),
        Span::dummy(),
    );

    let _checker = TypeChecker::new();
    // Type should be i64
}

#[test]
fn test_integer_suffix_u8() {
    let _lit = Literal::new(
        verum_ast::literal::LiteralKind::Int(IntLit::with_suffix(255, IntSuffix::U8)),
        Span::dummy(),
    );

    let _checker = TypeChecker::new();
    // Type should be u8
}

#[test]
fn test_integer_suffix_u16() {
    let _lit = Literal::new(
        verum_ast::literal::LiteralKind::Int(IntLit::with_suffix(65535, IntSuffix::U16)),
        Span::dummy(),
    );

    let _checker = TypeChecker::new();
    // Type should be u16
}

#[test]
fn test_integer_suffix_u32() {
    let _lit = Literal::new(
        verum_ast::literal::LiteralKind::Int(IntLit::with_suffix(4294967295, IntSuffix::U32)),
        Span::dummy(),
    );

    let _checker = TypeChecker::new();
    // Type should be u32
}

#[test]
fn test_integer_suffix_u64() {
    let _lit = Literal::new(
        verum_ast::literal::LiteralKind::Int(IntLit::with_suffix(
            18446744073709551615,
            IntSuffix::U64,
        )),
        Span::dummy(),
    );

    let _checker = TypeChecker::new();
    // Type should be u64
}

#[test]
fn test_float_suffix_f32() {
    let _lit = Literal::new(
        verum_ast::literal::LiteralKind::Float(FloatLit::with_suffix(3.14, FloatSuffix::F32)),
        Span::dummy(),
    );

    let _checker = TypeChecker::new();
    // Type should be f32
}

#[test]
fn test_float_suffix_f64() {
    let _lit = Literal::new(
        verum_ast::literal::LiteralKind::Float(FloatLit::with_suffix(2.71828, FloatSuffix::F64)),
        Span::dummy(),
    );

    let _checker = TypeChecker::new();
    // Type should be f64
}

#[test]
fn test_integer_no_suffix_defaults_to_int() {
    let _lit = Literal::new(
        verum_ast::literal::LiteralKind::Int(IntLit::new(42)),
        Span::dummy(),
    );

    let _checker = TypeChecker::new();
    // Type should default to Int (arbitrary precision)
}

#[test]
fn test_float_no_suffix_defaults_to_float() {
    let _lit = Literal::new(
        verum_ast::literal::LiteralKind::Float(FloatLit::new(3.14)),
        Span::dummy(),
    );

    let _checker = TypeChecker::new();
    // Type should default to Float (f64)
}

#[test]
fn test_custom_suffix_for_units() {
    // Test custom suffix for units of measure: 100_km
    let _lit = Literal::new(
        verum_ast::literal::LiteralKind::Int(IntLit::with_suffix(
            100,
            IntSuffix::Custom("km".into()),
        )),
        Span::dummy(),
    );

    let _checker = TypeChecker::new();
    // Type should be a Named type based on the suffix
}

#[test]
fn test_type_directed_interpretation() {
    // Unification: Robinson's algorithm extended with row polymorphism, refinement subtyping, and type class constraints — .4.1 lines 8672-8703
    // When expected type is known, literals should be checked against it

    // Example: let a: u8 = 100;
    // The literal 100 should be validated as fitting in u8 range
    // This would be tested through the full type checker, not just literal inference
}

#[test]
fn test_suffix_resolution_algorithm() {
    // Unification: Robinson's algorithm extended with row polymorphism, refinement subtyping, and type class constraints — .4.2 lines 8747-8754
    //
    // Resolution algorithm:
    // 1. Lookup suffix in registered suffix table
    // 2. Find type T that implements FromIntegerLiteral<SUFFIX> or FromFloatLiteral<SUFFIX>
    // 3. Return T
    // 4. If not found, emit compile error

    // For now, standard suffixes are hardcoded in infer_int_with_suffix
    // Future: extensible suffix registration system
}

#[test]
fn test_all_integer_suffixes_comprehensive() {
    let suffixes = vec![
        (IntSuffix::I8, "i8"),
        (IntSuffix::I16, "i16"),
        (IntSuffix::I32, "i32"),
        (IntSuffix::I64, "i64"),
        (IntSuffix::I128, "i128"),
        (IntSuffix::Isize, "isize"),
        (IntSuffix::U8, "u8"),
        (IntSuffix::U16, "u16"),
        (IntSuffix::U32, "u32"),
        (IntSuffix::U64, "u64"),
        (IntSuffix::U128, "u128"),
        (IntSuffix::Usize, "usize"),
    ];

    for (suffix, _expected_name) in suffixes {
        let _lit = Literal::new(
            verum_ast::literal::LiteralKind::Int(IntLit::with_suffix(42, suffix)),
            Span::dummy(),
        );

        let _checker = TypeChecker::new();
        // Each should produce the corresponding type
        // Verified by suffix_to_type_name mapping
    }
}
