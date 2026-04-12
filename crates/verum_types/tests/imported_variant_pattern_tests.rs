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
// Comprehensive tests for pattern matching with imported variant types
//
// Sum types (variants): "type T is A | B(payload) | C { fields }" for algebraic data types (Variants)
// Name resolution: deterministic lookup through module hierarchy, import resolution, re-exports — .1 - Cross-Module Type Resolution
//
// This test suite validates:
// 1. Pattern matching with imported Maybe.Some/Maybe.None
// 2. Pattern matching with imported Result.Ok/Result.Err
// 3. Nested pattern matching with imported variants
// 4. Complex variant patterns with cross-module types
// 5. Error cases for imported variant patterns
// 6. Multi-segment paths in variant patterns (std::Maybe::Some)

use indexmap::IndexMap;
use verum_ast::{
    MatchArm,
    expr::*,
    literal::Literal,
    pattern::{FieldPattern, Pattern, PatternKind, VariantPatternData},
    span::Span,
    ty::{Ident, Path, PathSegment},
};
use verum_common::{Heap, List, Text};
use verum_types::context::TypeScheme;
use verum_types::infer::*;
use verum_types::ty::Type;

// ============================================================================
// Test 1: Basic Imported Maybe Patterns
// ============================================================================

#[test]
fn test_match_imported_maybe_none() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Maybe<Int> = None | Some(Int) as if imported from std
    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("None"), Type::Unit);
    variants.insert(Text::from("Some"), Type::int());
    let maybe_int = Type::Variant(variants);

    // Pattern: std::Maybe::None (multi-segment path)
    let segments = vec![
        PathSegment::Name(Ident::new("std".to_string(), span)),
        PathSegment::Name(Ident::new("Maybe".to_string(), span)),
        PathSegment::Name(Ident::new("None".to_string(), span)),
    ];

    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::new(segments.into(), span),
            data: None, // No payload
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &maybe_int);
    assert!(
        result.is_ok(),
        "Imported Maybe::None pattern should match variant type"
    );
}

#[test]
fn test_match_imported_maybe_some() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Maybe<Text> = None | Some(Text) as imported type
    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("None"), Type::Unit);
    variants.insert(Text::from("Some"), Type::text());
    let maybe_text = Type::Variant(variants);

    // Pattern: std::Maybe::Some(value) (multi-segment path with binding)
    let segments = vec![
        PathSegment::Name(Ident::new("std".to_string(), span)),
        PathSegment::Name(Ident::new("Maybe".to_string(), span)),
        PathSegment::Name(Ident::new("Some".to_string(), span)),
    ];

    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::new(segments.into(), span),
            data: Some(VariantPatternData::Tuple(vec![Pattern::ident(
                Ident::new("value".to_string(), span),
                false,
                span,
            )].into())),
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &maybe_text);
    assert!(
        result.is_ok(),
        "Imported Maybe::Some(value) pattern should match"
    );

    // Verify value is bound to Text
    let value_scheme = checker.context_mut().env.lookup("value");
    assert!(value_scheme.is_some(), "value should be bound");
    assert_eq!(value_scheme.unwrap().ty, Type::text());
}

#[test]
fn test_match_imported_maybe_complete() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Maybe<Int> = None | Some(Int)
    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("None"), Type::Unit);
    variants.insert(Text::from("Some"), Type::int());
    let maybe_int = Type::Variant(variants.clone());

    // Create scrutinee: value
    checker
        .context_mut()
        .env
        .insert(Text::from("value"), TypeScheme::mono(maybe_int.clone()));

    let scrutinee = Expr::new(
        ExprKind::Path(Path::single(Ident::new("value".to_string(), span))),
        span,
    );

    // Pattern: Maybe.Some(x)
    let some_pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Some".to_string(), span)),
            data: Some(VariantPatternData::Tuple(vec![Pattern::ident(
                Ident::new("x".to_string(), span),
                false,
                span,
            )].into())),
        },
        span,
    );

    // Body: x + 1
    let x_expr = Expr::new(ExprKind::Path(Path::single(Ident::new("x", span))), span);
    let one_expr = Expr::literal(Literal::int(1, span));
    let some_body = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Box::new(x_expr),
            right: Box::new(one_expr),
        },
        span,
    );

    // Pattern: Maybe.None
    let none_pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("None".to_string(), span)),
            data: None,
        },
        span,
    );

    // Body: 0
    let none_body = Expr::literal(Literal::int(0, span));

    let match_expr = Expr::new(
        ExprKind::Match {
            expr: Box::new(scrutinee),
            arms: vec![
                MatchArm::new(some_pattern, None, Box::new(some_body), span),
                MatchArm::new(none_pattern, None, Box::new(none_body), span),
            ]
            .into(),
        },
        span,
    );

    let result = checker.synth_expr(&match_expr);
    assert!(
        result.is_ok(),
        "Match with imported Maybe patterns should type check"
    );
    assert_eq!(result.unwrap().ty, Type::int(), "Match should return Int");
}

// ============================================================================
// Test 2: Imported Result Patterns
// ============================================================================

#[test]
fn test_match_imported_result_ok() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Result<Int, Text> = Ok(Int) | Err(Text)
    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("Ok"), Type::int());
    variants.insert(Text::from("Err"), Type::text());
    let result_type = Type::Variant(variants);

    // Pattern: std::Result::Ok(value)
    let segments = vec![
        PathSegment::Name(Ident::new("std".to_string(), span)),
        PathSegment::Name(Ident::new("Result".to_string(), span)),
        PathSegment::Name(Ident::new("Ok".to_string(), span)),
    ];

    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::new(segments.into(), span),
            data: Some(VariantPatternData::Tuple(vec![Pattern::ident(
                Ident::new("value".to_string(), span),
                false,
                span,
            )].into())),
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &result_type);
    assert!(
        result.is_ok(),
        "Imported Result::Ok(value) pattern should match"
    );

    // Verify value is bound to Int
    let value_scheme = checker.context_mut().env.lookup("value");
    assert!(value_scheme.is_some(), "value should be bound");
    assert_eq!(value_scheme.unwrap().ty, Type::int());
}

#[test]
fn test_match_imported_result_err() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Result<Int, Text> = Ok(Int) | Err(Text)
    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("Ok"), Type::int());
    variants.insert(Text::from("Err"), Type::text());
    let result_type = Type::Variant(variants);

    // Pattern: std::Result::Err(error)
    let segments = vec![
        PathSegment::Name(Ident::new("std".to_string(), span)),
        PathSegment::Name(Ident::new("Result".to_string(), span)),
        PathSegment::Name(Ident::new("Err".to_string(), span)),
    ];

    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::new(segments.into(), span),
            data: Some(VariantPatternData::Tuple(vec![Pattern::ident(
                Ident::new("error".to_string(), span),
                false,
                span,
            )].into())),
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &result_type);
    assert!(
        result.is_ok(),
        "Imported Result::Err(error) pattern should match"
    );

    // Verify error is bound to Text
    let error_scheme = checker.context_mut().env.lookup("error");
    assert!(error_scheme.is_some(), "error should be bound");
    assert_eq!(error_scheme.unwrap().ty, Type::text());
}

#[test]
fn test_match_imported_result_complete() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Result<Int, Text> = Ok(Int) | Err(Text)
    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("Ok"), Type::int());
    variants.insert(Text::from("Err"), Type::text());
    let result_type = Type::Variant(variants.clone());

    // Create scrutinee: computation
    checker.context_mut().env.insert(
        Text::from("computation"),
        TypeScheme::mono(result_type.clone()),
    );

    let scrutinee = Expr::new(
        ExprKind::Path(Path::single(Ident::new("computation".to_string(), span))),
        span,
    );

    // Pattern: Result.Ok(n)
    let ok_pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Ok".to_string(), span)),
            data: Some(VariantPatternData::Tuple(vec![Pattern::ident(
                Ident::new("n".to_string(), span),
                false,
                span,
            )].into())),
        },
        span,
    );

    // Body: n * 2
    let n_expr = Expr::new(ExprKind::Path(Path::single(Ident::new("n", span))), span);
    let two_expr = Expr::literal(Literal::int(2, span));
    let ok_body = Expr::new(
        ExprKind::Binary {
            op: BinOp::Mul,
            left: Box::new(n_expr),
            right: Box::new(two_expr),
        },
        span,
    );

    // Pattern: Result.Err(msg)
    let err_pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Err".to_string(), span)),
            data: Some(VariantPatternData::Tuple(vec![Pattern::ident(
                Ident::new("msg".to_string(), span),
                false,
                span,
            )].into())),
        },
        span,
    );

    // Body: -1
    let err_body = Expr::literal(Literal::int(-1, span));

    let match_expr = Expr::new(
        ExprKind::Match {
            expr: Box::new(scrutinee),
            arms: vec![
                MatchArm::new(ok_pattern, None, Box::new(ok_body), span),
                MatchArm::new(err_pattern, None, Box::new(err_body), span),
            ]
            .into(),
        },
        span,
    );

    let result = checker.synth_expr(&match_expr);
    assert!(
        result.is_ok(),
        "Match with imported Result patterns should type check"
    );
    assert_eq!(result.unwrap().ty, Type::int(), "Match should return Int");
}

// ============================================================================
// Test 3: Nested Imported Variant Patterns
// ============================================================================

#[test]
fn test_match_nested_imported_variants_ok_some() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Maybe<Int> = None | Some(Int)
    let mut maybe_variants = IndexMap::new();
    maybe_variants.insert(Text::from("None"), Type::Unit);
    maybe_variants.insert(Text::from("Some"), Type::int());
    let maybe_int = Type::Variant(maybe_variants.clone());

    // Define Result<Maybe<Int>, Text> = Ok(Maybe<Int>) | Err(Text)
    let mut result_variants = IndexMap::new();
    result_variants.insert(Text::from("Ok"), maybe_int.clone());
    result_variants.insert(Text::from("Err"), Type::text());
    let result_maybe_int = Type::Variant(result_variants);

    // Pattern: Ok(Some(x)) - nested variant pattern with imported types
    let nested_pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Some".to_string(), span)),
            data: Some(VariantPatternData::Tuple(vec![Pattern::ident(
                Ident::new("x".to_string(), span),
                false,
                span,
            )].into())),
        },
        span,
    );

    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Ok".to_string(), span)),
            data: Some(VariantPatternData::Tuple(vec![nested_pattern].into())),
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &result_maybe_int);
    assert!(
        result.is_ok(),
        "Ok(Some(x)) nested imported pattern should match"
    );

    // Verify x is bound to Int
    let x_scheme = checker.context_mut().env.lookup("x");
    assert!(x_scheme.is_some(), "x should be bound");
    assert_eq!(x_scheme.unwrap().ty, Type::int());
}

#[test]
fn test_match_nested_imported_variants_ok_none() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Maybe<Int> = None | Some(Int)
    let mut maybe_variants = IndexMap::new();
    maybe_variants.insert(Text::from("None"), Type::Unit);
    maybe_variants.insert(Text::from("Some"), Type::int());
    let maybe_int = Type::Variant(maybe_variants.clone());

    // Define Result<Maybe<Int>, Text> = Ok(Maybe<Int>) | Err(Text)
    let mut result_variants = IndexMap::new();
    result_variants.insert(Text::from("Ok"), maybe_int.clone());
    result_variants.insert(Text::from("Err"), Type::text());
    let result_maybe_int = Type::Variant(result_variants);

    // Pattern: Ok(None) - nested variant pattern
    let nested_pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("None".to_string(), span)),
            data: None,
        },
        span,
    );

    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Ok".to_string(), span)),
            data: Some(VariantPatternData::Tuple(vec![nested_pattern].into())),
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &result_maybe_int);
    assert!(
        result.is_ok(),
        "Ok(None) nested imported pattern should match"
    );
}

#[test]
fn test_match_nested_imported_variants_complete() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Maybe<Int> = None | Some(Int)
    let mut maybe_variants = IndexMap::new();
    maybe_variants.insert(Text::from("None"), Type::Unit);
    maybe_variants.insert(Text::from("Some"), Type::int());
    let maybe_int = Type::Variant(maybe_variants.clone());

    // Define Result<Maybe<Int>, Text> = Ok(Maybe<Int>) | Err(Text)
    let mut result_variants = IndexMap::new();
    result_variants.insert(Text::from("Ok"), maybe_int.clone());
    result_variants.insert(Text::from("Err"), Type::text());
    let result_maybe_int = Type::Variant(result_variants.clone());

    // Create scrutinee: nested_result
    checker.context_mut().env.insert(
        Text::from("nested_result"),
        TypeScheme::mono(result_maybe_int.clone()),
    );

    let scrutinee = Expr::new(
        ExprKind::Path(Path::single(Ident::new("nested_result".to_string(), span))),
        span,
    );

    // Pattern: Ok(Some(value))
    let some_nested = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Some".to_string(), span)),
            data: Some(VariantPatternData::Tuple(vec![Pattern::ident(
                Ident::new("value".to_string(), span),
                false,
                span,
            )].into())),
        },
        span,
    );

    let ok_some_pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Ok".to_string(), span)),
            data: Some(VariantPatternData::Tuple(vec![some_nested].into())),
        },
        span,
    );

    // Body: value
    let value_expr = Expr::new(
        ExprKind::Path(Path::single(Ident::new("value", span))),
        span,
    );

    // Pattern: Ok(None)
    let none_nested = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("None".to_string(), span)),
            data: None,
        },
        span,
    );

    let ok_none_pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Ok".to_string(), span)),
            data: Some(VariantPatternData::Tuple(vec![none_nested].into())),
        },
        span,
    );

    // Body: 0
    let ok_none_body = Expr::literal(Literal::int(0, span));

    // Pattern: Err(msg)
    let err_pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Err".to_string(), span)),
            data: Some(VariantPatternData::Tuple(vec![Pattern::ident(
                Ident::new("msg".to_string(), span),
                false,
                span,
            )].into())),
        },
        span,
    );

    // Body: -1
    let err_body = Expr::literal(Literal::int(-1, span));

    let match_expr = Expr::new(
        ExprKind::Match {
            expr: Box::new(scrutinee),
            arms: vec![
                MatchArm::new(ok_some_pattern, None, Box::new(value_expr), span),
                MatchArm::new(ok_none_pattern, None, Box::new(ok_none_body), span),
                MatchArm::new(err_pattern, None, Box::new(err_body), span),
            ]
            .into(),
        },
        span,
    );

    let result = checker.synth_expr(&match_expr);
    assert!(
        result.is_ok(),
        "Match with nested imported variant patterns should type check"
    );
    assert_eq!(result.unwrap().ty, Type::int(), "Match should return Int");
}

// ============================================================================
// Test 4: Complex Imported Variant Patterns
// ============================================================================

#[test]
fn test_match_imported_variant_with_record_payload() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Error = Error { code: Int, message: Text }
    let mut error_fields = IndexMap::new();
    error_fields.insert(Text::from("code"), Type::int());
    error_fields.insert(Text::from("message"), Type::text());

    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("Error"), Type::Record(error_fields));
    let error_type = Type::Variant(variants);

    // Pattern: std::Error::Error { code, message }
    let segments = vec![
        PathSegment::Name(Ident::new("std".to_string(), span)),
        PathSegment::Name(Ident::new("Error".to_string(), span)),
        PathSegment::Name(Ident::new("Error".to_string(), span)),
    ];

    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::new(segments.into(), span),
            data: Some(VariantPatternData::Record {
                fields: vec![
                    FieldPattern::shorthand(Ident::new("code".to_string(), span)),
                    FieldPattern::shorthand(Ident::new("message".to_string(), span)),
                ]
                .into(),
                rest: false,
            }),
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &error_type);
    assert!(
        result.is_ok(),
        "Imported Error {{ code, message }} pattern should match"
    );

    // Verify bindings
    let code = checker.context_mut().env.lookup("code");
    assert!(code.is_some());
    assert_eq!(code.unwrap().ty, Type::int());

    let message = checker.context_mut().env.lookup("message");
    assert!(message.is_some());
    assert_eq!(message.unwrap().ty, Type::text());
}

#[test]
fn test_match_imported_variant_mixed_simple_paths() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Status = Pending | Success(Int) | Failure(Text)
    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("Pending"), Type::Unit);
    variants.insert(Text::from("Success"), Type::int());
    variants.insert(Text::from("Failure"), Type::text());
    let status_type = Type::Variant(variants);

    // Test all three patterns with simple paths (imported but last segment only)
    // Pattern 1: Pending
    let pending = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Pending".to_string(), span)),
            data: None,
        },
        span,
    );
    assert!(checker.bind_pattern(&pending, &status_type).is_ok());

    // Pattern 2: Success(value)
    let success = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Success".to_string(), span)),
            data: Some(VariantPatternData::Tuple(vec![Pattern::ident(
                Ident::new("value".to_string(), span),
                false,
                span,
            )].into())),
        },
        span,
    );
    assert!(checker.bind_pattern(&success, &status_type).is_ok());

    // Pattern 3: Failure(error)
    let failure = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Failure".to_string(), span)),
            data: Some(VariantPatternData::Tuple(vec![Pattern::ident(
                Ident::new("error".to_string(), span),
                false,
                span,
            )].into())),
        },
        span,
    );
    assert!(checker.bind_pattern(&failure, &status_type).is_ok());
}

// ============================================================================
// Test 5: Error Cases for Imported Variant Patterns
// ============================================================================

#[test]
fn test_match_imported_variant_unknown_constructor() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Maybe<Int> = None | Some(Int)
    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("None"), Type::Unit);
    variants.insert(Text::from("Some"), Type::int());
    let maybe_int = Type::Variant(variants);

    // Pattern: std::Maybe::Unknown - not a valid constructor
    let segments = vec![
        PathSegment::Name(Ident::new("std".to_string(), span)),
        PathSegment::Name(Ident::new("Maybe".to_string(), span)),
        PathSegment::Name(Ident::new("Unknown".to_string(), span)),
    ];

    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::new(segments.into(), span),
            data: None,
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &maybe_int);
    assert!(
        result.is_err(),
        "Unknown constructor in imported type should fail"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Unknown"),
        "Error should mention unknown constructor"
    );
}

#[test]
fn test_match_imported_variant_wrong_payload_type() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Maybe<Int> = None | Some(Int)
    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("None"), Type::Unit);
    variants.insert(Text::from("Some"), Type::int());
    let maybe_int = Type::Variant(variants);

    // Pattern: None(x) - ERROR: None has no payload
    let segments = vec![
        PathSegment::Name(Ident::new("std".to_string(), span)),
        PathSegment::Name(Ident::new("Maybe".to_string(), span)),
        PathSegment::Name(Ident::new("None".to_string(), span)),
    ];

    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::new(segments.into(), span),
            data: Some(VariantPatternData::Tuple(vec![Pattern::ident(
                Ident::new("x".to_string(), span),
                false,
                span,
            )].into())),
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &maybe_int);
    assert!(
        result.is_err(),
        "None with payload pattern on imported type should fail"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("has no payload"),
        "Error should mention no payload: {}",
        err_msg
    );
}

#[test]
fn test_match_imported_variant_missing_payload() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Maybe<Int> = None | Some(Int)
    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("None"), Type::Unit);
    variants.insert(Text::from("Some"), Type::int());
    let maybe_int = Type::Variant(variants);

    // Pattern: std::Maybe::Some — tag-only pattern (no payload destructuring).
    // This is intentionally accepted: Verum permits tag-only patterns for
    // type narrowing (`if x is Some`) and irrefutable match arms
    // (`match x { Some => ... }`). The payload is not bound but the
    // variant tag is matched.
    let segments = vec![
        PathSegment::Name(Ident::new("std".to_string(), span)),
        PathSegment::Name(Ident::new("Maybe".to_string(), span)),
        PathSegment::Name(Ident::new("Some".to_string(), span)),
    ];

    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::new(segments.into(), span),
            data: None, // Tag-only, no payload destructuring
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &maybe_int);
    assert!(
        result.is_ok(),
        "Tag-only variant pattern should be accepted for type narrowing"
    );
}

// ============================================================================
// Test 6: Multi-Segment Path Patterns
// ============================================================================

#[test]
fn test_match_deep_module_path_variant() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Maybe<Int> = None | Some(Int)
    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("None"), Type::Unit);
    variants.insert(Text::from("Some"), Type::int());
    let maybe_int = Type::Variant(variants);

    // Pattern: cog::std::collections::Maybe::Some(x) - deep module path
    let segments = vec![
        PathSegment::Name(Ident::new("cog".to_string(), span)),
        PathSegment::Name(Ident::new("std".to_string(), span)),
        PathSegment::Name(Ident::new("collections".to_string(), span)),
        PathSegment::Name(Ident::new("Maybe".to_string(), span)),
        PathSegment::Name(Ident::new("Some".to_string(), span)),
    ];

    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::new(segments.into(), span),
            data: Some(VariantPatternData::Tuple(vec![Pattern::ident(
                Ident::new("x".to_string(), span),
                false,
                span,
            )].into())),
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &maybe_int);
    assert!(
        result.is_ok(),
        "Deep module path pattern should work (using last segment as tag)"
    );

    // Verify x is bound
    assert!(checker.context_mut().env.lookup("x").is_some());
}

#[test]
fn test_match_absolute_vs_relative_paths() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Result<Int, Text> = Ok(Int) | Err(Text)
    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("Ok"), Type::int());
    variants.insert(Text::from("Err"), Type::text());
    let result_type = Type::Variant(variants.clone());

    // Pattern 1: Absolute path - cog::std::Result::Ok(x)
    let abs_segments = vec![
        PathSegment::Name(Ident::new("cog".to_string(), span)),
        PathSegment::Name(Ident::new("std".to_string(), span)),
        PathSegment::Name(Ident::new("Result".to_string(), span)),
        PathSegment::Name(Ident::new("Ok".to_string(), span)),
    ];

    let abs_pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::new(abs_segments.into(), span),
            data: Some(VariantPatternData::Tuple(vec![Pattern::ident(
                Ident::new("x".to_string(), span),
                false,
                span,
            )].into())),
        },
        span,
    );

    let result = checker.bind_pattern(&abs_pattern, &result_type);
    assert!(result.is_ok(), "Absolute path pattern should work");

    // Pattern 2: Relative path - Result::Ok(y)
    let rel_segments = vec![
        PathSegment::Name(Ident::new("Result".to_string(), span)),
        PathSegment::Name(Ident::new("Ok".to_string(), span)),
    ];

    let rel_pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::new(rel_segments.into(), span),
            data: Some(VariantPatternData::Tuple(vec![Pattern::ident(
                Ident::new("y".to_string(), span),
                false,
                span,
            )].into())),
        },
        span,
    );

    let result = checker.bind_pattern(&rel_pattern, &result_type);
    assert!(result.is_ok(), "Relative path pattern should work");
}
