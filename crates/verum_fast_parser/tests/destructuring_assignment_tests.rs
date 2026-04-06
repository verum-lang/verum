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
//! Tests for destructuring assignment parsing
//!
//! Tests for unified destructuring: tuple, array, record patterns in assignment position
//!
//! Destructuring assignment allows extracting components from compound values
//! and assigning them to multiple variables in a single expression:
//! - Tuple: `(a, b) = (b, a)` for swap operations
//! - Record: `Point { x, y } = compute_point()`
//! - Array: `[first, second, ..rest] = items`
//! - Compound: `(x, y) += (dx, dy)` for parallel updates

use verum_ast::pattern::PatternKind;
use verum_ast::{BinOp, Expr, ExprKind, FileId, Pattern};
use verum_fast_parser::VerumParser;

fn parse_expr(source: &str) -> Expr {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser
        .parse_expr_str(source, file_id)
        .unwrap_or_else(|e| panic!("Failed to parse '{}': {:?}", source, e))
}

fn parse_expr_should_fail(source: &str) -> bool {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser.parse_expr_str(source, file_id).is_err()
}

fn try_parse_expr(source: &str) -> Result<Expr, String> {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser.parse_expr_str(source, file_id)
        .map_err(|errors| {
            errors.iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        })
}

// === TUPLE DESTRUCTURING TESTS ===

#[test]
fn test_tuple_destructuring_simple() {
    // Basic tuple swap pattern
    let expr = parse_expr("(a, b) = (b, a)");
    match &expr.kind {
        ExprKind::DestructuringAssign { pattern, op, value } => {
            assert_eq!(*op, BinOp::Assign);

            // Check pattern is a tuple
            match &pattern.kind {
                PatternKind::Tuple(patterns) => {
                    assert_eq!(patterns.len(), 2, "Expected 2 elements in tuple pattern");
                    // Check first element is identifier 'a'
                    match &patterns[0].kind {
                        PatternKind::Ident { name, .. } => {
                            assert_eq!(name.as_str(), "a");
                        }
                        _ => panic!("Expected identifier pattern for first element"),
                    }
                    // Check second element is identifier 'b'
                    match &patterns[1].kind {
                        PatternKind::Ident { name, .. } => {
                            assert_eq!(name.as_str(), "b");
                        }
                        _ => panic!("Expected identifier pattern for second element"),
                    }
                }
                _ => panic!("Expected tuple pattern, got {:?}", pattern.kind),
            }

            // Check value is also a tuple
            match &value.kind {
                ExprKind::Tuple(exprs) => {
                    assert_eq!(exprs.len(), 2);
                }
                _ => panic!("Expected tuple expression on RHS"),
            }
        }
        _ => panic!("Expected DestructuringAssign, got {:?}", expr.kind),
    }
}

#[test]
fn test_tuple_destructuring_three_elements() {
    let expr = parse_expr("(x, y, z) = coords");
    match &expr.kind {
        ExprKind::DestructuringAssign { pattern, op, value } => {
            assert_eq!(*op, BinOp::Assign);
            match &pattern.kind {
                PatternKind::Tuple(patterns) => {
                    assert_eq!(patterns.len(), 3, "Expected 3 elements in tuple pattern");
                }
                _ => panic!("Expected tuple pattern"),
            }
        }
        _ => panic!("Expected DestructuringAssign"),
    }
}

#[test]
fn test_tuple_destructuring_with_wildcard() {
    // Ignore second element
    let expr = parse_expr("(first, _, last) = triple");
    match &expr.kind {
        ExprKind::DestructuringAssign { pattern, .. } => {
            match &pattern.kind {
                PatternKind::Tuple(patterns) => {
                    assert_eq!(patterns.len(), 3);
                    // Middle element should be wildcard
                    match &patterns[1].kind {
                        PatternKind::Wildcard => {}
                        _ => panic!("Expected wildcard pattern for middle element"),
                    }
                }
                _ => panic!("Expected tuple pattern"),
            }
        }
        _ => panic!("Expected DestructuringAssign"),
    }
}

#[test]
fn test_tuple_destructuring_nested() {
    // Nested tuple pattern
    let expr = parse_expr("((a, b), c) = nested");
    match &expr.kind {
        ExprKind::DestructuringAssign { pattern, .. } => {
            match &pattern.kind {
                PatternKind::Tuple(patterns) => {
                    assert_eq!(patterns.len(), 2);
                    // First element should be nested tuple
                    match &patterns[0].kind {
                        PatternKind::Tuple(inner) => {
                            assert_eq!(inner.len(), 2);
                        }
                        _ => panic!("Expected nested tuple pattern"),
                    }
                }
                _ => panic!("Expected tuple pattern"),
            }
        }
        _ => panic!("Expected DestructuringAssign"),
    }
}

#[test]
fn test_tuple_destructuring_single_element() {
    // Single element tuple (trailing comma makes it a tuple, not grouped)
    let expr = parse_expr("(x,) = single");
    match &expr.kind {
        ExprKind::DestructuringAssign { pattern, .. } => {
            match &pattern.kind {
                PatternKind::Tuple(patterns) => {
                    assert_eq!(patterns.len(), 1, "Single-element tuple should have 1 pattern");
                }
                _ => panic!("Expected tuple pattern"),
            }
        }
        _ => panic!("Expected DestructuringAssign"),
    }
}

// === ARRAY DESTRUCTURING TESTS ===

#[test]
fn test_array_destructuring_simple() {
    let expr = parse_expr("[first, second] = items");
    match &expr.kind {
        ExprKind::DestructuringAssign { pattern, op, .. } => {
            assert_eq!(*op, BinOp::Assign);
            match &pattern.kind {
                PatternKind::Array(patterns) => {
                    assert_eq!(patterns.len(), 2);
                }
                _ => panic!("Expected array pattern, got {:?}", pattern.kind),
            }
        }
        _ => panic!("Expected DestructuringAssign"),
    }
}

#[test]
fn test_array_destructuring_with_rest() {
    let expr = parse_expr("[head, ..tail] = list");
    match &expr.kind {
        ExprKind::DestructuringAssign { pattern, .. } => {
            match &pattern.kind {
                PatternKind::Array(patterns) => {
                    assert_eq!(patterns.len(), 2);
                    // First should be identifier
                    match &patterns[0].kind {
                        PatternKind::Ident { name, .. } => {
                            assert_eq!(name.as_str(), "head");
                        }
                        _ => panic!("Expected identifier for first element"),
                    }
                    // Second should be rest pattern (possibly with binding using @ syntax)
                    match &patterns[1].kind {
                        PatternKind::Rest => {
                            // Rest without binding
                        }
                        PatternKind::Ident { name, subpattern, .. } => {
                            // ..tail becomes an ident pattern
                            assert_eq!(name.as_str(), "tail");
                        }
                        _ => panic!("Expected rest pattern or identifier, got {:?}", patterns[1].kind),
                    }
                }
                _ => panic!("Expected array pattern"),
            }
        }
        _ => panic!("Expected DestructuringAssign"),
    }
}

#[test]
fn test_array_destructuring_middle_rest() {
    let expr = parse_expr("[first, ..middle, last] = items");
    match &expr.kind {
        ExprKind::DestructuringAssign { pattern, .. } => {
            match &pattern.kind {
                PatternKind::Array(patterns) => {
                    assert_eq!(patterns.len(), 3);
                }
                _ => panic!("Expected array pattern"),
            }
        }
        _ => panic!("Expected DestructuringAssign"),
    }
}

#[test]
fn test_array_destructuring_ignore_rest() {
    // Discard remaining elements
    let expr = parse_expr("[first, ..] = items");
    match &expr.kind {
        ExprKind::DestructuringAssign { pattern, .. } => {
            match &pattern.kind {
                PatternKind::Array(patterns) => {
                    assert_eq!(patterns.len(), 2);
                    // Second should be rest without binding
                    match &patterns[1].kind {
                        PatternKind::Rest => {}
                        _ => panic!("Expected unbound rest pattern, got {:?}", patterns[1].kind),
                    }
                }
                _ => panic!("Expected array pattern"),
            }
        }
        _ => panic!("Expected DestructuringAssign"),
    }
}

// === RECORD DESTRUCTURING TESTS ===

#[test]
fn test_record_destructuring_simple() {
    let expr = parse_expr("Point { x, y } = point");
    match &expr.kind {
        ExprKind::DestructuringAssign { pattern, op, .. } => {
            assert_eq!(*op, BinOp::Assign);
            match &pattern.kind {
                PatternKind::Record { path, fields, .. } => {
                    // Check type name
                    assert_eq!(path.segments.len(), 1);
                    assert_eq!(fields.len(), 2);
                }
                _ => panic!("Expected record pattern, got {:?}", pattern.kind),
            }
        }
        _ => panic!("Expected DestructuringAssign"),
    }
}

#[test]
fn test_record_destructuring_with_rename() {
    // Field rename: `x: local_x` binds field `x` to `local_x`
    let expr = parse_expr("Point { x: local_x, y: local_y } = point");
    match &expr.kind {
        ExprKind::DestructuringAssign { pattern, .. } => {
            match &pattern.kind {
                PatternKind::Record { fields, .. } => {
                    assert_eq!(fields.len(), 2);
                    // Check that fields have patterns (renamed)
                    assert!(fields[0].pattern.is_some());
                }
                _ => panic!("Expected record pattern"),
            }
        }
        _ => panic!("Expected DestructuringAssign"),
    }
}

#[test]
fn test_record_destructuring_partial() {
    // Partial destructuring - just list the fields you want:
    let expr = parse_expr("Config { timeout } = config");
    match &expr.kind {
        ExprKind::DestructuringAssign { pattern, .. } => {
            match &pattern.kind {
                PatternKind::Record { fields, rest, .. } => {
                    assert_eq!(fields.len(), 1);
                    assert!(!rest, "Should not have rest pattern");
                }
                _ => panic!("Expected record pattern"),
            }
        }
        _ => panic!("Expected DestructuringAssign, got {:?}", expr.kind),
    }
}

#[test]
fn test_record_destructuring_with_rest() {
    // Rest pattern: { x, y, .. } extracts fields and ignores rest
    let expr = parse_expr("Config { timeout, .. } = config");
    match &expr.kind {
        ExprKind::DestructuringAssign { pattern, .. } => {
            match &pattern.kind {
                PatternKind::Record { fields, rest, .. } => {
                    assert_eq!(fields.len(), 1, "Should have 1 field");
                    assert!(*rest, "Should have rest pattern (..)");
                }
                _ => panic!("Expected record pattern"),
            }
        }
        _ => panic!("Expected DestructuringAssign, got {:?}", expr.kind),
    }
}

#[test]
fn test_record_destructuring_multiple_fields_with_rest() {
    // Multiple fields with rest: { x, y, .. }
    let expr = parse_expr("Point { x, y, .. } = point");
    match &expr.kind {
        ExprKind::DestructuringAssign { pattern, .. } => {
            match &pattern.kind {
                PatternKind::Record { fields, rest, .. } => {
                    assert_eq!(fields.len(), 2, "Should have 2 fields");
                    assert!(*rest, "Should have rest pattern (..)");
                }
                _ => panic!("Expected record pattern"),
            }
        }
        _ => panic!("Expected DestructuringAssign, got {:?}", expr.kind),
    }
}

#[test]
fn test_record_struct_update_rejected_in_destructuring() {
    // Struct update syntax { ..base } is NOT allowed in destructuring
    let result = try_parse_expr("Point { x, ..other } = point");
    assert!(result.is_err(), "Struct update should be rejected in destructuring");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("struct update syntax") ||
        err.to_string().contains("not allowed"),
        "Error should mention struct update syntax: {}", err
    );
}

#[test]
fn test_record_destructuring_qualified_path() {
    let expr = parse_expr("std.io.Config { host, port } = config");
    match &expr.kind {
        ExprKind::DestructuringAssign { pattern, .. } => {
            match &pattern.kind {
                PatternKind::Record { path, fields, .. } => {
                    assert!(path.segments.len() > 1, "Expected qualified path");
                    assert_eq!(fields.len(), 2);
                }
                _ => panic!("Expected record pattern"),
            }
        }
        _ => panic!("Expected DestructuringAssign"),
    }
}

// === COMPOUND ASSIGNMENT TESTS ===

#[test]
fn test_tuple_compound_add_assign() {
    let expr = parse_expr("(x, y) += (dx, dy)");
    match &expr.kind {
        ExprKind::DestructuringAssign { pattern, op, value } => {
            assert_eq!(*op, BinOp::AddAssign, "Expected += operator");
            match &pattern.kind {
                PatternKind::Tuple(patterns) => {
                    assert_eq!(patterns.len(), 2);
                }
                _ => panic!("Expected tuple pattern"),
            }
        }
        _ => panic!("Expected DestructuringAssign, got {:?}", expr.kind),
    }
}

#[test]
fn test_tuple_compound_sub_assign() {
    let expr = parse_expr("(x, y) -= (1, 1)");
    match &expr.kind {
        ExprKind::DestructuringAssign { op, .. } => {
            assert_eq!(*op, BinOp::SubAssign, "Expected -= operator");
        }
        _ => panic!("Expected DestructuringAssign"),
    }
}

#[test]
fn test_tuple_compound_mul_assign() {
    let expr = parse_expr("(a, b) *= (2, 2)");
    match &expr.kind {
        ExprKind::DestructuringAssign { op, .. } => {
            assert_eq!(*op, BinOp::MulAssign, "Expected *= operator");
        }
        _ => panic!("Expected DestructuringAssign"),
    }
}

#[test]
fn test_array_compound_assign() {
    let expr = parse_expr("[x, y, z] *= scale");
    match &expr.kind {
        ExprKind::DestructuringAssign { pattern, op, .. } => {
            assert_eq!(*op, BinOp::MulAssign);
            match &pattern.kind {
                PatternKind::Array(patterns) => {
                    assert_eq!(patterns.len(), 3);
                }
                _ => panic!("Expected array pattern"),
            }
        }
        _ => panic!("Expected DestructuringAssign"),
    }
}

// === COMPLEX PATTERNS ===

#[test]
fn test_mixed_nested_destructuring() {
    // Nested array inside tuple
    let expr = parse_expr("(header, [first, ..rest]) = packet");
    match &expr.kind {
        ExprKind::DestructuringAssign { pattern, .. } => {
            match &pattern.kind {
                PatternKind::Tuple(patterns) => {
                    assert_eq!(patterns.len(), 2);
                    // Second element should be array pattern
                    match &patterns[1].kind {
                        PatternKind::Array(_) => {}
                        _ => panic!("Expected nested array pattern"),
                    }
                }
                _ => panic!("Expected tuple pattern"),
            }
        }
        _ => panic!("Expected DestructuringAssign"),
    }
}

#[test]
fn test_destructuring_with_function_call() {
    // RHS is function call
    let expr = parse_expr("(min, max) = compute_bounds(data)");
    match &expr.kind {
        ExprKind::DestructuringAssign { value, .. } => {
            match &value.kind {
                ExprKind::Call { .. } => {}
                _ => panic!("Expected call expression on RHS"),
            }
        }
        _ => panic!("Expected DestructuringAssign"),
    }
}

#[test]
fn test_destructuring_with_method_call() {
    // RHS is method call
    let expr = parse_expr("(x, y) = point.to_tuple()");
    match &expr.kind {
        ExprKind::DestructuringAssign { value, .. } => {
            match &value.kind {
                ExprKind::MethodCall { .. } => {}
                _ => panic!("Expected method call expression on RHS"),
            }
        }
        _ => panic!("Expected DestructuringAssign"),
    }
}

#[test]
fn test_destructuring_with_binary_expr() {
    // RHS is binary expression (arithmetic, not pipeline since pipeline binds lower)
    let expr = parse_expr("(a, b) = (x + 1, y + 2)");
    match &expr.kind {
        ExprKind::DestructuringAssign { value, .. } => {
            // Value should be a tuple of binary expressions
            match &value.kind {
                ExprKind::Tuple(elements) => {
                    assert_eq!(elements.len(), 2);
                }
                _ => panic!("Expected tuple on RHS"),
            }
        }
        _ => panic!("Expected DestructuringAssign, got {:?}", expr.kind),
    }
}

// === EDGE CASES ===

#[test]
fn test_empty_tuple_pattern() {
    // Empty tuple - unit assignment
    let expr = parse_expr("() = unit_value");
    match &expr.kind {
        ExprKind::DestructuringAssign { pattern, .. } => {
            match &pattern.kind {
                PatternKind::Tuple(patterns) => {
                    assert!(patterns.is_empty(), "Expected empty tuple pattern");
                }
                _ => panic!("Expected tuple pattern"),
            }
        }
        _ => panic!("Expected DestructuringAssign"),
    }
}

#[test]
fn test_deeply_nested_tuple() {
    let expr = parse_expr("(((a,),),) = deep");
    match &expr.kind {
        ExprKind::DestructuringAssign { pattern, .. } => {
            // Verify it's deeply nested tuples
            let mut current = &pattern.kind;
            let mut depth = 0;
            loop {
                match current {
                    PatternKind::Tuple(patterns) if patterns.len() == 1 => {
                        depth += 1;
                        current = &patterns[0].kind;
                    }
                    PatternKind::Ident { .. } => break,
                    _ => break,
                }
            }
            assert!(depth >= 3, "Expected at least 3 levels of nesting");
        }
        _ => panic!("Expected DestructuringAssign"),
    }
}

// === ASSIGNMENT TARGET VALIDATION ===
// These verify that non-assignable patterns are rejected

#[test]
fn test_literal_pattern_not_destructuring() {
    // Numeric literal on LHS should NOT produce DestructuringAssign
    // It should be an error - literals aren't valid assignment targets
    assert!(
        parse_expr_should_fail("42 = x"),
        "Literal on LHS of assignment should be a parse error"
    );
}

// === PARENTHESIZED EXPRESSIONS ===

#[test]
fn test_parenthesized_single_ident_not_tuple() {
    // (x) without trailing comma is NOT a tuple, just grouping
    // So `(x) = y` should be regular assignment, not destructuring
    let expr = parse_expr("(x) = y");
    match &expr.kind {
        ExprKind::Binary { op: BinOp::Assign, .. } => {
            // Good - this is regular assignment
        }
        ExprKind::DestructuringAssign { pattern, .. } => {
            // Also acceptable if we treat parens as grouping
            match &pattern.kind {
                PatternKind::Ident { .. } | PatternKind::Paren(_) => {}
                _ => panic!("Parenthesized single ident should not be tuple pattern"),
            }
        }
        _ => panic!("Expected assignment expression, got {:?}", expr.kind),
    }
}

// === FAILURE TESTS ===
// These tests verify that invalid destructuring syntax is correctly rejected

#[test]
fn test_function_call_not_valid_destructuring_target() {
    // Function calls cannot be destructuring targets
    assert!(
        parse_expr_should_fail("foo() = x"),
        "Function call should not be valid LHS"
    );
}

#[test]
fn test_method_call_not_valid_destructuring_target() {
    // Method calls cannot be destructuring targets
    assert!(
        parse_expr_should_fail("obj.method() = x"),
        "Method call should not be valid LHS"
    );
}

#[test]
fn test_binary_expr_not_valid_destructuring_target() {
    // Binary expressions cannot be destructuring targets
    assert!(
        parse_expr_should_fail("(a + b) = x"),
        "Binary expression should not be valid LHS"
    );
}

#[test]
fn test_string_literal_not_valid_destructuring_target() {
    // String literals cannot be destructuring targets
    assert!(
        parse_expr_should_fail("\"hello\" = x"),
        "String literal should not be valid LHS"
    );
}

#[test]
fn test_tuple_with_literal_element_invalid() {
    // Tuples containing literals cannot be destructuring targets
    assert!(
        parse_expr_should_fail("(a, 42) = x"),
        "Tuple with literal element should not be valid"
    );
}

#[test]
fn test_tuple_with_function_call_element_invalid() {
    // Tuples containing function calls cannot be destructuring targets
    assert!(
        parse_expr_should_fail("(a, foo()) = x"),
        "Tuple with function call element should not be valid"
    );
}

#[test]
fn test_array_with_literal_element_invalid() {
    // Arrays containing literals cannot be destructuring targets
    assert!(
        parse_expr_should_fail("[a, 42] = x"),
        "Array with literal element should not be valid"
    );
}

#[test]
fn test_nested_invalid_pattern_rejected() {
    // Nested invalid patterns should be rejected
    assert!(
        parse_expr_should_fail("((a, foo()), b) = x"),
        "Nested function call should not be valid"
    );
}

#[test]
fn test_record_with_invalid_field_pattern() {
    // Record patterns with invalid field patterns should be rejected
    assert!(
        parse_expr_should_fail("Point { x: foo(), y } = p"),
        "Record with function call in field pattern should not be valid"
    );
}
