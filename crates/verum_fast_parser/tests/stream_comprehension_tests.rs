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
// Comprehensive test suite for stream comprehension parsing in Verum.
//
// This test suite covers all stream comprehension syntax as per grammar/verum.ebnf:
// - Basic stream comprehension: stream[x * 2 for x in items]
// - Stream with filter: stream[x for x in items if x > 0]
// - Stream with let binding: stream[y for x in items let y = x * 2]
// - Nested stream comprehension
// - Stream with multiple for clauses
// - Stream with type annotations in let
// - Complex stream comprehension scenarios
//
// Grammar Reference:
// stream_comprehension_expr = 'stream' , '[' , stream_body , ']' ;
// stream_body = expression , 'for' , pattern , 'in' , expression , { stream_clause } ;
// stream_clause = 'for' , pattern , 'in' , expression
//               | 'let' , pattern , [ ':' , type_expr ] , '=' , expression
//               | 'if' , expression ;

use verum_ast::expr::{ComprehensionClause, ComprehensionClauseKind};
use verum_ast::{Expr, ExprKind, FileId, PatternKind};
use verum_fast_parser::VerumParser;

/// Helper function to parse an expression from a string.
fn parse_expr(source: &str) -> Result<Expr, String> {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser.parse_expr_str(source, file_id).map_err(|errors| {
        errors
            .into_iter()
            .map(|e| format!("{:?}", e))
            .collect::<Vec<_>>()
            .join(", ")
    })
}

/// Helper to check if parsing succeeds and return the expression.
fn assert_parses(source: &str) -> Expr {
    parse_expr(source).unwrap_or_else(|_| panic!("Failed to parse: {}", source))
}

/// Helper to check if parsing fails.
fn assert_fails(source: &str) {
    assert!(
        parse_expr(source).is_err(),
        "Expected parse failure for: {}",
        source
    );
}

/// Extract stream comprehension from parsed expression.
fn extract_stream_comprehension(expr: &Expr) -> (&Expr, &[ComprehensionClause]) {
    match &expr.kind {
        ExprKind::StreamComprehension { expr, clauses } => (expr.as_ref(), clauses.as_ref()),
        _ => panic!("Expected StreamComprehension, got {:?}", expr.kind),
    }
}

// === BASIC STREAM COMPREHENSION TESTS ===

#[test]
fn test_basic_stream_comprehension() {
    let source = "stream[x * 2 for x in items]";
    let expr = assert_parses(source);

    let (output_expr, clauses) = extract_stream_comprehension(&expr);

    // Verify output expression (x * 2)
    match &output_expr.kind {
        ExprKind::Binary { .. } => {} // Multiplication
        _ => panic!("Expected binary expression for output"),
    }

    // Verify single for clause
    assert_eq!(clauses.len(), 1);
    match &clauses[0].kind {
        ComprehensionClauseKind::For { pattern, iter } => {
            // Pattern should be identifier 'x'
            match &pattern.kind {
                PatternKind::Ident { name, .. } => {
                    assert_eq!(name.name.as_str(), "x");
                }
                _ => panic!("Expected identifier pattern"),
            }

            // Iterator should be 'items'
            match &iter.kind {
                ExprKind::Path(_) => {}
                _ => panic!("Expected path expression for iterator"),
            }
        }
        _ => panic!("Expected For clause"),
    }
}

#[test]
fn test_stream_comprehension_simple_identity() {
    let source = "stream[x for x in data]";
    let expr = assert_parses(source);

    let (output_expr, clauses) = extract_stream_comprehension(&expr);

    // Output should be identifier 'x'
    match &output_expr.kind {
        ExprKind::Path(_) => {}
        _ => panic!("Expected path expression for output"),
    }

    assert_eq!(clauses.len(), 1);
}

#[test]
fn test_stream_comprehension_range() {
    let source = "stream[x for x in 0..100]";
    let expr = assert_parses(source);

    let (_, clauses) = extract_stream_comprehension(&expr);
    assert_eq!(clauses.len(), 1);

    match &clauses[0].kind {
        ComprehensionClauseKind::For { iter, .. } => {
            // Verify iterator is a range expression
            match &iter.kind {
                ExprKind::Range { .. } => {}
                _ => panic!("Expected range expression"),
            }
        }
        _ => panic!("Expected For clause"),
    }
}

// === STREAM WITH FILTER TESTS ===

#[test]
fn test_stream_with_single_filter() {
    let source = "stream[x for x in items if x > 0]";
    let expr = assert_parses(source);

    let (_, clauses) = extract_stream_comprehension(&expr);

    // Should have 2 clauses: for and if
    assert_eq!(clauses.len(), 2);

    match &clauses[0].kind {
        ComprehensionClauseKind::For { .. } => {}
        _ => panic!("Expected For clause as first clause"),
    }

    match &clauses[1].kind {
        ComprehensionClauseKind::If(_) => {}
        _ => panic!("Expected If clause as second clause"),
    }
}

#[test]
fn test_stream_with_multiple_filters() {
    let source = "stream[x for x in items if x > 0 if x < 100]";
    let expr = assert_parses(source);

    let (_, clauses) = extract_stream_comprehension(&expr);

    // Should have 3 clauses: for, if, if
    assert_eq!(clauses.len(), 3);

    match &clauses[0].kind {
        ComprehensionClauseKind::For { .. } => {}
        _ => panic!("Expected For clause"),
    }

    match &clauses[1].kind {
        ComprehensionClauseKind::If(_) => {}
        _ => panic!("Expected first If clause"),
    }

    match &clauses[2].kind {
        ComprehensionClauseKind::If(_) => {}
        _ => panic!("Expected second If clause"),
    }
}

#[test]
fn test_stream_with_complex_filter() {
    let source = "stream[x for x in items if x % 2 == 0]";
    let expr = assert_parses(source);

    let (_, clauses) = extract_stream_comprehension(&expr);
    assert_eq!(clauses.len(), 2);
}

// === STREAM WITH LET BINDING TESTS ===

#[test]
fn test_stream_with_let_binding() {
    let source = "stream[y for x in items let y = x * 2]";
    let expr = assert_parses(source);

    let (_, clauses) = extract_stream_comprehension(&expr);

    // Should have 2 clauses: for and let
    assert_eq!(clauses.len(), 2);

    match &clauses[0].kind {
        ComprehensionClauseKind::For { .. } => {}
        _ => panic!("Expected For clause"),
    }

    match &clauses[1].kind {
        ComprehensionClauseKind::Let { pattern, ty, value } => {
            // Pattern should be 'y'
            match &pattern.kind {
                PatternKind::Ident { name, .. } => {
                    assert_eq!(name.name.as_str(), "y");
                }
                _ => panic!("Expected identifier pattern"),
            }

            // No type annotation
            assert!(ty.is_none());

            // Value should be binary expression (x * 2)
            match &value.kind {
                ExprKind::Binary { .. } => {}
                _ => panic!("Expected binary expression for let value"),
            }
        }
        _ => panic!("Expected Let clause"),
    }
}

#[test]
fn test_stream_with_typed_let_binding() {
    let source = "stream[y for x in items let y: Int = x * 2]";
    let expr = assert_parses(source);

    let (_, clauses) = extract_stream_comprehension(&expr);
    assert_eq!(clauses.len(), 2);

    match &clauses[1].kind {
        ComprehensionClauseKind::Let { ty, .. } => {
            // Should have type annotation
            assert!(ty.is_some());
        }
        _ => panic!("Expected Let clause"),
    }
}

#[test]
fn test_stream_with_multiple_let_bindings() {
    let source = "stream[z for x in items let y = x * 2 let z = y + 1]";
    let expr = assert_parses(source);

    let (_, clauses) = extract_stream_comprehension(&expr);

    // Should have 3 clauses: for, let, let
    assert_eq!(clauses.len(), 3);

    match &clauses[0].kind {
        ComprehensionClauseKind::For { .. } => {}
        _ => panic!("Expected For clause"),
    }

    match &clauses[1].kind {
        ComprehensionClauseKind::Let { .. } => {}
        _ => panic!("Expected first Let clause"),
    }

    match &clauses[2].kind {
        ComprehensionClauseKind::Let { .. } => {}
        _ => panic!("Expected second Let clause"),
    }
}

// === STREAM WITH MULTIPLE FOR CLAUSES TESTS ===

#[test]
fn test_stream_with_multiple_for_clauses() {
    let source = "stream[(x, y) for x in rows for y in cols]";
    let expr = assert_parses(source);

    let (output_expr, clauses) = extract_stream_comprehension(&expr);

    // Output should be tuple (x, y)
    match &output_expr.kind {
        ExprKind::Tuple { .. } => {}
        _ => panic!("Expected tuple expression for output"),
    }

    // Should have 2 for clauses
    assert_eq!(clauses.len(), 2);

    match &clauses[0].kind {
        ComprehensionClauseKind::For { pattern, .. } => match &pattern.kind {
            PatternKind::Ident { name, .. } => {
                assert_eq!(name.name.as_str(), "x");
            }
            _ => panic!("Expected identifier pattern for x"),
        },
        _ => panic!("Expected first For clause"),
    }

    match &clauses[1].kind {
        ComprehensionClauseKind::For { pattern, .. } => match &pattern.kind {
            PatternKind::Ident { name, .. } => {
                assert_eq!(name.name.as_str(), "y");
            }
            _ => panic!("Expected identifier pattern for y"),
        },
        _ => panic!("Expected second For clause"),
    }
}

#[test]
fn test_stream_cartesian_product_with_filter() {
    let source = "stream[(x, y) for x in 1..10 for y in 1..10 if x + y == 10]";
    let expr = assert_parses(source);

    let (_, clauses) = extract_stream_comprehension(&expr);

    // Should have 3 clauses: for, for, if
    assert_eq!(clauses.len(), 3);
}

// === COMPLEX STREAM COMPREHENSION TESTS ===

#[test]
fn test_stream_with_mixed_clauses() {
    let source = "stream[result for x in items let y = x * 2 if y > 0 let result = y + 1]";
    let expr = assert_parses(source);

    let (_, clauses) = extract_stream_comprehension(&expr);

    // Should have 4 clauses: for, let, if, let
    assert_eq!(clauses.len(), 4);

    match &clauses[0].kind {
        ComprehensionClauseKind::For { .. } => {}
        _ => panic!("Expected For clause"),
    }

    match &clauses[1].kind {
        ComprehensionClauseKind::Let { .. } => {}
        _ => panic!("Expected first Let clause"),
    }

    match &clauses[2].kind {
        ComprehensionClauseKind::If(_) => {}
        _ => panic!("Expected If clause"),
    }

    match &clauses[3].kind {
        ComprehensionClauseKind::Let { .. } => {}
        _ => panic!("Expected second Let clause"),
    }
}

#[test]
fn test_stream_with_pattern_destructuring() {
    let source = "stream[x + y for (x, y) in pairs]";
    let expr = assert_parses(source);

    let (_, clauses) = extract_stream_comprehension(&expr);
    assert_eq!(clauses.len(), 1);

    match &clauses[0].kind {
        ComprehensionClauseKind::For { pattern, .. } => {
            // Pattern should be tuple pattern (x, y)
            match &pattern.kind {
                PatternKind::Tuple { .. } => {}
                _ => panic!("Expected tuple pattern"),
            }
        }
        _ => panic!("Expected For clause"),
    }
}

#[test]
fn test_stream_with_record_pattern() {
    let source = "stream[x + y for Point { x, y } in points]";
    let expr = assert_parses(source);

    let (_, clauses) = extract_stream_comprehension(&expr);
    assert_eq!(clauses.len(), 1);

    match &clauses[0].kind {
        ComprehensionClauseKind::For { pattern, .. } => match &pattern.kind {
            PatternKind::Record { .. } => {}
            _ => panic!("Expected record pattern"),
        },
        _ => panic!("Expected For clause"),
    }
}

#[test]
fn test_stream_with_complex_output_expression() {
    let source = "stream[{ x: x, y: y, sum: x + y } for x in 1..10 for y in 1..10]";
    let expr = assert_parses(source);

    let (output_expr, _) = extract_stream_comprehension(&expr);

    // Output should be a record or map expression (depending on parser interpretation)
    match &output_expr.kind {
        ExprKind::Record { .. } => {}
        ExprKind::MapLiteral { .. } => {} // Parser might interpret as map literal
        _ => panic!(
            "Expected record or map expression for output, got {:?}",
            output_expr.kind
        ),
    }
}

#[test]
fn test_stream_with_function_call() {
    let source = "stream[process(x) for x in items]";
    let expr = assert_parses(source);

    let (output_expr, _) = extract_stream_comprehension(&expr);

    match &output_expr.kind {
        ExprKind::Call { .. } => {}
        _ => panic!("Expected call expression for output"),
    }
}

#[test]
fn test_stream_with_method_call() {
    let source = "stream[x.toString() for x in items]";
    let expr = assert_parses(source);

    let (output_expr, _) = extract_stream_comprehension(&expr);

    match &output_expr.kind {
        ExprKind::MethodCall { .. } => {}
        _ => panic!("Expected method call expression for output"),
    }
}

// === NESTED STREAM COMPREHENSION TESTS ===

#[test]
fn test_nested_stream_comprehension_in_iterator() {
    // Stream comprehension where the iterator is itself a stream comprehension
    let source = "stream[x for x in stream[y * 2 for y in items]]";
    let expr = assert_parses(source);

    let (_, clauses) = extract_stream_comprehension(&expr);
    assert_eq!(clauses.len(), 1);

    match &clauses[0].kind {
        ComprehensionClauseKind::For { iter, .. } => {
            // Iterator should be a stream comprehension
            match &iter.kind {
                ExprKind::StreamComprehension { .. } => {}
                _ => panic!("Expected nested stream comprehension"),
            }
        }
        _ => panic!("Expected For clause"),
    }
}

#[test]
fn test_nested_stream_comprehension_in_output() {
    // Stream comprehension where the output is itself a stream comprehension
    let source = "stream[stream[y for y in inner] for inner in outer]";
    let expr = assert_parses(source);

    let (output_expr, _) = extract_stream_comprehension(&expr);

    // Output should be a stream comprehension
    match &output_expr.kind {
        ExprKind::StreamComprehension { .. } => {}
        _ => panic!("Expected nested stream comprehension in output"),
    }
}

#[test]
fn test_nested_stream_comprehension_in_let() {
    let source = "stream[processed for x in items let processed = stream[y for y in x]]";
    let expr = assert_parses(source);

    let (_, clauses) = extract_stream_comprehension(&expr);
    assert_eq!(clauses.len(), 2);

    match &clauses[1].kind {
        ComprehensionClauseKind::Let { value, .. } => match &value.kind {
            ExprKind::StreamComprehension { .. } => {}
            _ => panic!("Expected stream comprehension in let value"),
        },
        _ => panic!("Expected Let clause"),
    }
}

// === EDGE CASES AND ERROR TESTS ===

#[test]
fn test_stream_empty_is_not_comprehension() {
    // Without content, 'stream[]' is parsed as subscript with empty array literal
    // not as a stream comprehension - this is correct parser behavior
    let expr = assert_parses("stream[]");
    match &expr.kind {
        ExprKind::StreamComprehension { .. } => {
            panic!("Should not parse as stream comprehension without 'for' clause")
        }
        _ => {} // Any non-stream-comprehension parsing is acceptable
    }
}

#[test]
fn test_stream_without_for_is_subscript() {
    // Without 'for', 'stream[x]' is parsed as subscript expression (stream indexed by x)
    // not as a stream comprehension - this is correct parser behavior
    let expr = assert_parses("stream[x]");
    match &expr.kind {
        ExprKind::StreamComprehension { .. } => {
            panic!("Should not parse as stream comprehension without 'for' clause")
        }
        ExprKind::Index { .. } => {} // This is expected - subscript expression
        other => {
            // Accept any non-stream-comprehension parsing
            // (could be Call, Index, etc. depending on parser)
        }
    }
}

#[test]
fn test_stream_missing_in_fails() {
    // 'for' clause must have 'in'
    assert_fails("stream[x for x]");
}

#[test]
fn test_stream_unclosed_fails() {
    // Missing closing bracket
    assert_fails("stream[x for x in items");
}

#[test]
fn test_stream_with_trailing_comma_fails() {
    // Stream comprehensions don't use commas
    assert_fails("stream[x for x in items,]");
}

// === COMPARISON WITH REGULAR COMPREHENSION ===

#[test]
fn test_regular_comprehension_vs_stream() {
    // Regular list comprehension
    let list_comp = assert_parses("[x * 2 for x in items]");
    match &list_comp.kind {
        ExprKind::Comprehension { .. } => {}
        _ => panic!("Expected regular comprehension"),
    }

    // Stream comprehension
    let stream_comp = assert_parses("stream[x * 2 for x in items]");
    match &stream_comp.kind {
        ExprKind::StreamComprehension { .. } => {}
        _ => panic!("Expected stream comprehension"),
    }
}

// === PRACTICAL EXAMPLES ===

#[test]
fn test_stream_fibonacci_like() {
    let source = "stream[a + b for (a, b) in pairs]";
    assert_parses(source);
}

#[test]
fn test_stream_filtering_even_numbers() {
    let source = "stream[x for x in 0..1000 if x % 2 == 0]";
    assert_parses(source);
}

#[test]
fn test_stream_pythagorean_triples() {
    let source = "stream[(a, b, c) for a in 1..100 for b in a..100 for c in b..100 if a * a + b * b == c * c]";
    assert_parses(source);
}

#[test]
fn test_stream_with_pipeline() {
    // Stream comprehension with pipeline operator in output
    let source = "stream[x |> process |> validate for x in items]";
    assert_parses(source);
}

#[test]
fn test_stream_with_async_calls() {
    // Stream comprehension with await in output
    let source = "stream[item.fetch().await for item in items]";
    assert_parses(source);
}

#[test]
fn test_stream_matrix_transpose() {
    let source = "stream[stream[matrix[i][j] for i in 0..rows] for j in 0..cols]";
    assert_parses(source);
}

#[test]
fn test_stream_with_type_annotation_in_pattern() {
    // Type annotation in let binding
    let source = "stream[doubled for x in items let doubled: Int = x * 2]";
    assert_parses(source);
}

#[test]
fn test_stream_with_wildcard_pattern() {
    let source = "stream[1 for _ in 0..10]";
    let expr = assert_parses(source);

    let (_, clauses) = extract_stream_comprehension(&expr);
    match &clauses[0].kind {
        ComprehensionClauseKind::For { pattern, .. } => match &pattern.kind {
            PatternKind::Wildcard => {}
            _ => panic!("Expected wildcard pattern"),
        },
        _ => panic!("Expected For clause"),
    }
}

#[test]
fn test_stream_with_rest_pattern() {
    let source = "stream[first for [first, ..] in lists]";
    let expr = assert_parses(source);

    let (_, clauses) = extract_stream_comprehension(&expr);
    match &clauses[0].kind {
        ComprehensionClauseKind::For { pattern, .. } => {
            // Pattern should be array or slice pattern with rest
            match &pattern.kind {
                PatternKind::Array { .. } => {}
                PatternKind::Slice { .. } => {}
                _ => panic!("Expected array or slice pattern, got {:?}", pattern.kind),
            }
        }
        _ => panic!("Expected For clause"),
    }
}

#[test]
fn test_stream_flatmap_pattern() {
    // Flattening nested structures
    let source = "stream[item for list in lists for item in list]";
    assert_parses(source);
}

#[test]
fn test_stream_zip_pattern() {
    let source = "stream[(x, y) for (x, y) in zip(list1, list2)]";
    assert_parses(source);
}
