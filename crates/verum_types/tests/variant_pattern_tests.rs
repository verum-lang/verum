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
// Comprehensive tests for variant pattern matching (P0 Blocker #10)
//
// Sum types (variants): "type T is A | B(payload) | C { fields }" for algebraic data types (Variants)
//
// This test suite validates:
// 1. Simple variant patterns (None, Some)
// 2. Tuple-style variants: Ok(x), Error(code, msg)
// 3. Record-style variants: Person { name, age }
// 4. Nested patterns
// 5. Error cases (unknown variant, wrong payload type, missing fields)

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
// Test 1: Simple Variant Patterns (Unit Payloads)
// ============================================================================

#[test]
fn test_variant_pattern_unit_none() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Maybe<Int> = None | Some(Int)
    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("None"), Type::Unit);
    variants.insert(Text::from("Some"), Type::int());
    let maybe_int = Type::Variant(variants);

    // Pattern: None
    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("None".to_string(), span)),
            data: None, // No payload
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &maybe_int);
    assert!(result.is_ok(), "None pattern should match variant type");
}

#[test]
fn test_variant_pattern_unit_with_payload_error() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Maybe<Int> = None | Some(Int)
    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("None"), Type::Unit);
    variants.insert(Text::from("Some"), Type::int());
    let maybe_int = Type::Variant(variants);

    // Pattern: None(x) - ERROR: None has no payload
    let tuple_list = vec![Pattern::ident(
        Ident::new("x".to_string(), span),
        false,
        span,
    )];
    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("None".to_string(), span)),
            data: Some(VariantPatternData::Tuple(tuple_list.into())),
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &maybe_int);
    assert!(result.is_err(), "None with payload pattern should fail");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("has no payload"),
        "Error should mention no payload: {}",
        err_msg
    );
}

// ============================================================================
// Test 2: Simple Tuple-Style Variants
// ============================================================================

#[test]
fn test_variant_pattern_tuple_some() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Maybe<Int> = None | Some(Int)
    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("None"), Type::Unit);
    variants.insert(Text::from("Some"), Type::int());
    let maybe_int = Type::Variant(variants);

    // Pattern: Some(x)
    let tuple_list = vec![Pattern::ident(
        Ident::new("x".to_string(), span),
        false,
        span,
    )];
    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Some".to_string(), span)),
            data: Some(VariantPatternData::Tuple(tuple_list.into())),
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &maybe_int);
    assert!(result.is_ok(), "Some(x) pattern should match");

    // Verify x is bound to Int
    let x_scheme = checker.context_mut().env.lookup("x");
    assert!(x_scheme.is_some(), "x should be bound");
    assert_eq!(x_scheme.unwrap().ty, Type::int());
}

#[test]
fn test_variant_pattern_tuple_multi_field() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Result<Int, Text> = Ok(Int) | Err(Text)
    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("Ok"), Type::int());
    variants.insert(Text::from("Err"), Type::text());
    let result_type = Type::Variant(variants);

    // Pattern: Ok(value)
    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Ok".to_string(), span)),
            data: Some(VariantPatternData::Tuple(vec![Pattern::ident(
                Ident::new("value".to_string(), span),
                false,
                span,
            )].into())),
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &result_type);
    assert!(result.is_ok(), "Ok(value) pattern should match");

    // Verify value is bound to Int
    let value_scheme = checker.context_mut().env.lookup("value");
    assert!(value_scheme.is_some(), "value should be bound");
    assert_eq!(value_scheme.unwrap().ty, Type::int());
}

#[test]
fn test_variant_pattern_tuple_multiple_args() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Point = Point(Int, Int, Int)
    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(
        Text::from("Point"),
        Type::tuple(vec![Type::int(), Type::int(), Type::int()].into()),
    );
    let point_type = Type::Variant(variants);

    // Pattern: Point(x, y, z)
    let tuple_list = vec![
        Pattern::ident(Ident::new("x".to_string(), span), false, span),
        Pattern::ident(Ident::new("y".to_string(), span), false, span),
        Pattern::ident(Ident::new("z".to_string(), span), false, span),
    ];
    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Point".to_string(), span)),
            data: Some(VariantPatternData::Tuple(tuple_list.into())),
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &point_type);
    assert!(result.is_ok(), "Point(x, y, z) pattern should match");

    // Verify all bindings
    assert!(checker.context_mut().env.lookup("x").is_some());
    assert!(checker.context_mut().env.lookup("y").is_some());
    assert!(checker.context_mut().env.lookup("z").is_some());
}

// ============================================================================
// Test 3: Record-Style Variant Patterns
// ============================================================================

#[test]
fn test_variant_pattern_record_simple() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Error = Error { code: Int, message: Text }
    let mut error_fields = IndexMap::new();
    error_fields.insert(Text::from("code"), Type::int());
    error_fields.insert(Text::from("message"), Type::text());

    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("Error"), Type::Record(error_fields));
    let error_type = Type::Variant(variants);

    // Pattern: Error { code, message }
    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Error".to_string(), span)),
            data: Some(VariantPatternData::Record {
                fields: vec![
                    FieldPattern::shorthand(Ident::new("code".to_string(), span)),
                    FieldPattern::shorthand(Ident::new("message".to_string(), span)),
                ].into(),
                rest: false,
            }),
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &error_type);
    assert!(
        result.is_ok(),
        "Error {{ code, message }} pattern should match"
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
fn test_variant_pattern_record_explicit_bindings() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Person = Person { name: Text, age: Int }
    let mut person_fields = IndexMap::new();
    person_fields.insert(Text::from("name"), Type::text());
    person_fields.insert(Text::from("age"), Type::int());

    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("Person"), Type::Record(person_fields));
    let person_type = Type::Variant(variants);

    // Pattern: Person { name: n, age: a }
    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Person".to_string(), span)),
            data: Some(VariantPatternData::Record {
                fields: vec![
                    FieldPattern::new(
                        Ident::new("name".to_string(), span),
                        Some(Pattern::ident(
                            Ident::new("n".to_string(), span),
                            false,
                            span,
                        )),
                        span,
                    ),
                    FieldPattern::new(
                        Ident::new("age".to_string(), span),
                        Some(Pattern::ident(
                            Ident::new("a".to_string(), span),
                            false,
                            span,
                        )),
                        span,
                    ),
                ].into(),
                rest: false,
            }),
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &person_type);
    assert!(
        result.is_ok(),
        "Person {{ name: n, age: a }} pattern should match"
    );

    // Verify bindings use new names
    assert!(checker.context_mut().env.lookup("n").is_some());
    assert!(checker.context_mut().env.lookup("a").is_some());
    assert!(
        checker.context_mut().env.lookup("name").is_none(),
        "name should not be bound"
    );
    assert!(
        checker.context_mut().env.lookup("age").is_none(),
        "age should not be bound"
    );
}

#[test]
fn test_variant_pattern_record_with_rest() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Data = Data { x: Int, y: Int, z: Int }
    let mut data_fields = IndexMap::new();
    data_fields.insert(Text::from("x"), Type::int());
    data_fields.insert(Text::from("y"), Type::int());
    data_fields.insert(Text::from("z"), Type::int());

    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("Data"), Type::Record(data_fields));
    let data_type = Type::Variant(variants);

    // Pattern: Data { x, .. } - only match x, ignore rest
    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Data".to_string(), span)),
            data: Some(VariantPatternData::Record {
                fields: vec![FieldPattern::shorthand(Ident::new("x", span))].into(),
                rest: true, // Allow unmatched fields
            }),
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &data_type);
    assert!(result.is_ok(), "Data {{ x, .. }} pattern should match");

    // Only x should be bound
    assert!(checker.context_mut().env.lookup("x").is_some());
}

// ============================================================================
// Test 4: Nested Patterns
// ============================================================================

#[test]
fn test_variant_pattern_nested() {
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
    let result_type = Type::Variant(result_variants);

    // Pattern: Ok(Some(x)) - nested variant pattern
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

    let result = checker.bind_pattern(&pattern, &result_type);
    assert!(result.is_ok(), "Ok(Some(x)) nested pattern should match");

    // Verify x is bound
    assert!(checker.context_mut().env.lookup("x").is_some());
}

// ============================================================================
// Test 5: Error Cases
// ============================================================================

#[test]
fn test_variant_pattern_unknown_constructor() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Maybe<Int> = None | Some(Int)
    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("None"), Type::Unit);
    variants.insert(Text::from("Some"), Type::int());
    let maybe_int = Type::Variant(variants);

    // Pattern: Unknown - not a valid constructor
    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Unknown".to_string(), span)),
            data: None,
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &maybe_int);
    assert!(result.is_err(), "Unknown constructor should fail");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Unknown"),
        "Error should mention unknown constructor"
    );
    assert!(
        err_msg.contains("None"),
        "Error should list available variants"
    );
    assert!(
        err_msg.contains("Some"),
        "Error should list available variants"
    );
}

#[test]
fn test_variant_pattern_wrong_arity() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Point = Point(Int, Int, Int)
    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(
        Text::from("Point"),
        Type::tuple(vec![Type::int(), Type::int(), Type::int()].into()),
    );
    let point_type = Type::Variant(variants);

    // Pattern: Point(x, y) - wrong arity (expects 3, got 2)
    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Point".to_string(), span)),
            data: Some(VariantPatternData::Tuple(vec![
                Pattern::ident(Ident::new("x".to_string(), span), false, span),
                Pattern::ident(Ident::new("y".to_string(), span), false, span),
            ].into())),
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &point_type);
    assert!(result.is_err(), "Wrong arity should fail");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("expects 3"),
        "Error should mention expected arity"
    );
    assert!(
        err_msg.contains("has 2"),
        "Error should mention actual arity"
    );
}

#[test]
fn test_variant_pattern_record_missing_field() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Error = Error { code: Int, message: Text }
    let mut error_fields = IndexMap::new();
    error_fields.insert(Text::from("code"), Type::int());
    error_fields.insert(Text::from("message"), Type::text());

    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("Error"), Type::Record(error_fields));
    let error_type = Type::Variant(variants);

    // Pattern: Error { code } - missing 'message' field
    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Error".to_string(), span)),
            data: Some(VariantPatternData::Record {
                fields: vec![FieldPattern::shorthand(Ident::new("code", span))].into(),
                rest: false, // No rest pattern
            }),
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &error_type);
    assert!(result.is_err(), "Missing required field should fail");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("message"),
        "Error should mention missing field"
    );
}

#[test]
fn test_variant_pattern_record_unknown_field() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Error = Error { code: Int, message: Text }
    let mut error_fields = IndexMap::new();
    error_fields.insert(Text::from("code"), Type::int());
    error_fields.insert(Text::from("message"), Type::text());

    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("Error"), Type::Record(error_fields));
    let error_type = Type::Variant(variants);

    // Pattern: Error { code, unknown } - 'unknown' is not a field
    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Error".to_string(), span)),
            data: Some(VariantPatternData::Record {
                fields: vec![
                    FieldPattern::shorthand(Ident::new("code".to_string(), span)),
                    FieldPattern::shorthand(Ident::new("message".to_string(), span)),
                    FieldPattern::shorthand(Ident::new("unknown".to_string(), span)),
                ].into(),
                rest: false,
            }),
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &error_type);
    assert!(result.is_err(), "Unknown field should fail");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("unknown"),
        "Error should mention unknown field"
    );
    assert!(
        err_msg.contains("code"),
        "Error should list available fields"
    );
    assert!(
        err_msg.contains("message"),
        "Error should list available fields"
    );
}

#[test]
fn test_variant_pattern_not_variant_type() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Pattern: Some(x) but scrutinee is just Int (not a variant)
    let pattern = Pattern::new(
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

    let result = checker.bind_pattern(&pattern, &Type::int());
    assert!(
        result.is_err(),
        "Variant pattern on non-variant type should fail"
    );
    let err_msg = result.unwrap_err().to_string();
    // The error message format may vary - check for any reasonable error message
    // about the type mismatch (variant pattern on non-variant type)
    assert!(
        err_msg.contains("variant") || err_msg.contains("Int") || err_msg.contains("Some"),
        "Error should indicate pattern/type mismatch: {}", err_msg
    );
}

#[test]
fn test_variant_pattern_record_style_on_tuple_payload() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Point = Point(Int, Int)
    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(
        Text::from("Point"),
        Type::tuple(vec![Type::int(), Type::int()].into()),
    );
    let point_type = Type::Variant(variants);

    // Pattern: Point { x, y } - trying record-style on tuple payload
    let pattern = Pattern::new(
        PatternKind::Variant {
            path: Path::single(Ident::new("Point".to_string(), span)),
            data: Some(VariantPatternData::Record {
                fields: vec![
                    FieldPattern::shorthand(Ident::new("x", span)),
                    FieldPattern::shorthand(Ident::new("y", span)),
                ].into(),
                rest: false,
            }),
        },
        span,
    );

    let result = checker.bind_pattern(&pattern, &point_type);
    assert!(
        result.is_err(),
        "Record pattern on tuple payload should fail"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("not a record"),
        "Error should mention payload is not a record"
    );
}

// ============================================================================
// Test 6: Integration with Match Expressions
// ============================================================================

#[test]
fn test_variant_pattern_in_match_expression() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Maybe<Int> = None | Some(Int)
    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("None"), Type::Unit);
    variants.insert(Text::from("Some"), Type::int());
    let maybe_int = Type::Variant(variants);

    // match value {
    //     Some(x) => x + 1,
    //     None => 0
    // }

    // Create scrutinee: value
    checker
        .context_mut()
        .env
        .insert(Text::from("value"), TypeScheme::mono(maybe_int.clone()));

    let scrutinee = Expr::new(
        ExprKind::Path(Path::single(Ident::new("value".to_string(), span))),
        span,
    );

    // Pattern: Some(x)
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

    // Pattern: None
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
            ].into(),
        },
        span,
    );

    let result = checker.synth_expr(&match_expr);
    assert!(
        result.is_ok(),
        "Match with variant patterns should type check"
    );
    assert_eq!(result.unwrap().ty, Type::int(), "Match should return Int");
}

#[test]
fn test_variant_pattern_multi_segment_path() {
    let mut checker = TypeChecker::new();
    let span = Span::dummy();

    // Define Maybe<Int> = None | Some(Int)
    let mut variants: IndexMap<Text, Type> = IndexMap::new();
    variants.insert(Text::from("None"), Type::Unit);
    variants.insert(Text::from("Some"), Type::int());
    let maybe_int = Type::Variant(variants);

    // Pattern: std::Some(x) - multi-segment path
    let segments = vec![
        PathSegment::Name(Ident::new("std".to_string(), span)),
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
        "Multi-segment path should work (using last segment as tag)"
    );
}
