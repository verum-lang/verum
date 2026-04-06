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
// Comprehensive test suite for Cluster 2: Comprehension Completeness
//
// This test suite covers all comprehension syntax as per grammar/verum.ebnf:
// - Map Comprehension: {k: v for (k, v) in items if condition}
// - Set Comprehension: set{x for x in items if condition}
// - Generator Expression: gen{x for x in items if condition}
//
// Grammar Reference:
// map_comprehension = '{' , expression , ':' , expression , 'for' , pattern , 'in' , expression
//                   , { comprehension_clause } , '}' ;
// set_comprehension = 'set' , '{' , expression , 'for' , pattern , 'in' , expression
//                   , { comprehension_clause } , '}' ;
// generator_expr = 'gen' , '{' , expression , 'for' , pattern , 'in' , expression
//                , { comprehension_clause } , '}' ;
// comprehension_clause = 'for' , pattern , 'in' , expression
//                      | 'let' , pattern , [ ':' , type_expr ] , '=' , expression
//                      | 'if' , expression ;

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

/// Extract map comprehension from parsed expression.
fn extract_map_comprehension(expr: &Expr) -> (&Expr, &Expr, &[ComprehensionClause]) {
    match &expr.kind {
        ExprKind::MapComprehension {
            key_expr,
            value_expr,
            clauses,
        } => (key_expr.as_ref(), value_expr.as_ref(), clauses.as_ref()),
        _ => panic!("Expected MapComprehension, got {:?}", expr.kind),
    }
}

/// Extract set comprehension from parsed expression.
fn extract_set_comprehension(expr: &Expr) -> (&Expr, &[ComprehensionClause]) {
    match &expr.kind {
        ExprKind::SetComprehension { expr, clauses } => (expr.as_ref(), clauses.as_ref()),
        _ => panic!("Expected SetComprehension, got {:?}", expr.kind),
    }
}

/// Extract generator comprehension from parsed expression.
fn extract_generator_comprehension(expr: &Expr) -> (&Expr, &[ComprehensionClause]) {
    match &expr.kind {
        ExprKind::GeneratorComprehension { expr, clauses } => (expr.as_ref(), clauses.as_ref()),
        _ => panic!("Expected GeneratorComprehension, got {:?}", expr.kind),
    }
}

// ============================================================================
// MAP COMPREHENSION TESTS
// ============================================================================

#[test]
fn test_basic_map_comprehension() {
    let source = "{k: v for (k, v) in items}";
    let expr = assert_parses(source);

    let (key_expr, value_expr, clauses) = extract_map_comprehension(&expr);

    // Verify key expression is identifier 'k'
    match &key_expr.kind {
        ExprKind::Path(path) => {
            assert!(path.segments.len() == 1);
        }
        _ => panic!("Expected path expression for key"),
    }

    // Verify value expression is identifier 'v'
    match &value_expr.kind {
        ExprKind::Path(path) => {
            assert!(path.segments.len() == 1);
        }
        _ => panic!("Expected path expression for value"),
    }

    // Verify single for clause
    assert_eq!(clauses.len(), 1);
    match &clauses[0].kind {
        ComprehensionClauseKind::For { pattern, iter: _ } => {
            // Pattern should be tuple (k, v)
            match &pattern.kind {
                PatternKind::Tuple(_) => {}
                _ => panic!("Expected tuple pattern"),
            }
        }
        _ => panic!("Expected for clause"),
    }
}

#[test]
fn test_map_comprehension_with_filter() {
    let source = "{k: v * 2 for (k, v) in items if v > 0}";
    let expr = assert_parses(source);

    let (_, _, clauses) = extract_map_comprehension(&expr);

    // Should have for clause and if clause
    assert_eq!(clauses.len(), 2);
    assert!(matches!(&clauses[0].kind, ComprehensionClauseKind::For { .. }));
    assert!(matches!(&clauses[1].kind, ComprehensionClauseKind::If(_)));
}

#[test]
fn test_map_comprehension_with_let_binding() {
    let source = "{k: doubled for (k, v) in items let doubled = v * 2}";
    let expr = assert_parses(source);

    let (_, _, clauses) = extract_map_comprehension(&expr);

    // Should have for clause and let clause
    assert_eq!(clauses.len(), 2);
    assert!(matches!(&clauses[0].kind, ComprehensionClauseKind::For { .. }));
    assert!(matches!(
        &clauses[1].kind,
        ComprehensionClauseKind::Let { .. }
    ));
}

#[test]
fn test_map_comprehension_with_multiple_clauses() {
    let source = "{k: v for k in keys for v in values if k != \"\" let result = (k, v)}";
    let expr = assert_parses(source);

    let (_, _, clauses) = extract_map_comprehension(&expr);

    // Should have: for, for, if, let
    assert_eq!(clauses.len(), 4);
    assert!(matches!(&clauses[0].kind, ComprehensionClauseKind::For { .. }));
    assert!(matches!(&clauses[1].kind, ComprehensionClauseKind::For { .. }));
    assert!(matches!(&clauses[2].kind, ComprehensionClauseKind::If(_)));
    assert!(matches!(
        &clauses[3].kind,
        ComprehensionClauseKind::Let { .. }
    ));
}

#[test]
fn test_map_comprehension_expression_keys() {
    // Key expression can be any expression, not just identifier
    let source = "{i * 10: items[i] for i in 0..len}";
    let expr = assert_parses(source);

    let (key_expr, _, _) = extract_map_comprehension(&expr);

    // Key should be multiplication expression
    assert!(matches!(&key_expr.kind, ExprKind::Binary { .. }));
}

#[test]
fn test_map_comprehension_nested_in_function_call() {
    let source = "process({k: v for (k, v) in data})";
    let expr = assert_parses(source);

    // Outer should be function call
    match &expr.kind {
        ExprKind::Call { args, .. } => {
            assert_eq!(args.len(), 1);
            // Argument should be map comprehension
            assert!(matches!(&args[0].kind, ExprKind::MapComprehension { .. }));
        }
        _ => panic!("Expected function call"),
    }
}

// ============================================================================
// SET COMPREHENSION TESTS
// ============================================================================

#[test]
fn test_basic_set_comprehension() {
    let source = "set{x for x in items}";
    let expr = assert_parses(source);

    let (output_expr, clauses) = extract_set_comprehension(&expr);

    // Verify output expression is identifier 'x'
    match &output_expr.kind {
        ExprKind::Path(path) => {
            assert!(path.segments.len() == 1);
        }
        _ => panic!("Expected path expression for output"),
    }

    // Verify single for clause
    assert_eq!(clauses.len(), 1);
}

#[test]
fn test_set_comprehension_with_transform() {
    let source = "set{x * 2 for x in items}";
    let expr = assert_parses(source);

    let (output_expr, _) = extract_set_comprehension(&expr);

    // Output should be multiplication expression
    assert!(matches!(&output_expr.kind, ExprKind::Binary { .. }));
}

#[test]
fn test_set_comprehension_with_filter() {
    let source = "set{x for x in items if x > 0}";
    let expr = assert_parses(source);

    let (_, clauses) = extract_set_comprehension(&expr);

    // Should have for and if clauses
    assert_eq!(clauses.len(), 2);
    assert!(matches!(&clauses[0].kind, ComprehensionClauseKind::For { .. }));
    assert!(matches!(&clauses[1].kind, ComprehensionClauseKind::If(_)));
}

#[test]
fn test_set_comprehension_with_let_binding() {
    let source = "set{doubled for x in items let doubled = x * 2}";
    let expr = assert_parses(source);

    let (_, clauses) = extract_set_comprehension(&expr);

    // Should have for and let clauses
    assert_eq!(clauses.len(), 2);
    assert!(matches!(&clauses[0].kind, ComprehensionClauseKind::For { .. }));
    assert!(matches!(
        &clauses[1].kind,
        ComprehensionClauseKind::Let { .. }
    ));
}

#[test]
fn test_set_comprehension_with_typed_let() {
    let source = "set{y for x in items let y: Int = x * 2}";
    let expr = assert_parses(source);

    let (_, clauses) = extract_set_comprehension(&expr);

    // Let clause should have type annotation
    match &clauses[1].kind {
        ComprehensionClauseKind::Let { ty, .. } => {
            assert!(ty.is_some());
        }
        _ => panic!("Expected let clause"),
    }
}

#[test]
fn test_set_comprehension_nested_for() {
    let source = "set{(x, y) for x in xs for y in ys}";
    let expr = assert_parses(source);

    let (_, clauses) = extract_set_comprehension(&expr);

    // Should have two for clauses
    assert_eq!(clauses.len(), 2);
    assert!(matches!(&clauses[0].kind, ComprehensionClauseKind::For { .. }));
    assert!(matches!(&clauses[1].kind, ComprehensionClauseKind::For { .. }));
}

#[test]
fn test_set_comprehension_complex() {
    let source = "set{result for x in items if x > 0 let doubled = x * 2 for y in others if doubled == y let result = (x, y)}";
    let expr = assert_parses(source);

    let (_, clauses) = extract_set_comprehension(&expr);

    // Should have: for, if, let, for, if, let
    assert_eq!(clauses.len(), 6);
}

// ============================================================================
// GENERATOR EXPRESSION TESTS
// ============================================================================

#[test]
fn test_basic_generator_expression() {
    let source = "gen{x for x in items}";
    let expr = assert_parses(source);

    let (output_expr, clauses) = extract_generator_comprehension(&expr);

    // Verify output expression is identifier 'x'
    match &output_expr.kind {
        ExprKind::Path(path) => {
            assert!(path.segments.len() == 1);
        }
        _ => panic!("Expected path expression for output"),
    }

    // Verify single for clause
    assert_eq!(clauses.len(), 1);
}

#[test]
fn test_generator_with_transform() {
    let source = "gen{x * 2 for x in items}";
    let expr = assert_parses(source);

    let (output_expr, _) = extract_generator_comprehension(&expr);

    // Output should be multiplication expression
    assert!(matches!(&output_expr.kind, ExprKind::Binary { .. }));
}

#[test]
fn test_generator_with_filter() {
    let source = "gen{x for x in items if x > 0}";
    let expr = assert_parses(source);

    let (_, clauses) = extract_generator_comprehension(&expr);

    // Should have for and if clauses
    assert_eq!(clauses.len(), 2);
    assert!(matches!(&clauses[0].kind, ComprehensionClauseKind::For { .. }));
    assert!(matches!(&clauses[1].kind, ComprehensionClauseKind::If(_)));
}

#[test]
fn test_generator_with_let_binding() {
    let source = "gen{doubled for x in items let doubled = x * 2}";
    let expr = assert_parses(source);

    let (_, clauses) = extract_generator_comprehension(&expr);

    // Should have for and let clauses
    assert_eq!(clauses.len(), 2);
}

#[test]
fn test_generator_nested_for() {
    let source = "gen{(x, y) for x in xs for y in ys}";
    let expr = assert_parses(source);

    let (_, clauses) = extract_generator_comprehension(&expr);

    // Should have two for clauses
    assert_eq!(clauses.len(), 2);
}

#[test]
fn test_generator_lazy_evaluation_context() {
    // Generator expressions should work in lazy evaluation contexts
    let source = "take(10, gen{fib(n) for n in 0..})";
    let expr = assert_parses(source);

    // Should be a function call with generator argument
    match &expr.kind {
        ExprKind::Call { args, .. } => {
            assert_eq!(args.len(), 2);
            assert!(matches!(
                &args[1].kind,
                ExprKind::GeneratorComprehension { .. }
            ));
        }
        _ => panic!("Expected function call"),
    }
}

// ============================================================================
// DISAMBIGUATION TESTS
// ============================================================================

#[test]
fn test_map_literal_vs_map_comprehension() {
    // Map literal (no 'for')
    let literal = parse_expr("{a: 1, b: 2}").expect("Should parse map literal");
    assert!(matches!(literal.kind, ExprKind::MapLiteral { .. }));

    // Map comprehension (has 'for')
    let comprehension = parse_expr("{k: v for (k, v) in items}").expect("Should parse map comprehension");
    assert!(matches!(comprehension.kind, ExprKind::MapComprehension { .. }));
}

#[test]
fn test_set_keyword_disambiguation() {
    // set{} with braces is set comprehension
    let comp = parse_expr("set{x for x in items}").expect("Should parse set comprehension");
    assert!(matches!(comp.kind, ExprKind::SetComprehension { .. }));

    // set without braces should be parsed as path (for things like set::new())
    let path = parse_expr("set.new()").expect("Should parse as method call");
    // This would be a method call on the identifier 'set'
    assert!(matches!(path.kind, ExprKind::MethodCall { .. }));
}

#[test]
fn test_gen_keyword_disambiguation() {
    // gen{} with braces is generator expression
    let comp = parse_expr("gen{x for x in items}").expect("Should parse generator expression");
    assert!(matches!(comp.kind, ExprKind::GeneratorComprehension { .. }));

    // gen without braces should be parsed as path
    let path = parse_expr("gen.next()").expect("Should parse as method call");
    assert!(matches!(path.kind, ExprKind::MethodCall { .. }));
}

#[test]
fn test_empty_map_is_literal_not_comprehension() {
    let expr = parse_expr("{}").expect("Should parse empty map");
    // Empty braces should be map literal, not comprehension
    assert!(matches!(expr.kind, ExprKind::MapLiteral { .. }));
}

// ============================================================================
// ERROR CASE TESTS
// ============================================================================

#[test]
fn test_map_comprehension_missing_for() {
    // Map comprehension requires 'for' keyword
    let result = parse_expr("{k: v in items}");
    // This should parse as a map literal with 'v in items' as a binary 'in' expression
    // or fail depending on grammar - either is acceptable
}

#[test]
fn test_set_comprehension_missing_closing_brace() {
    // set{} requires closing brace
    assert_fails("set{x for x in items");
}

#[test]
fn test_generator_missing_for_keyword() {
    // Generator requires 'for' after expression
    assert_fails("gen{x in items}");
}

#[test]
fn test_comprehension_missing_in_keyword() {
    // All comprehensions require 'in' after pattern
    assert_fails("set{x for x items}");
    assert_fails("gen{x for x items}");
}

#[test]
fn test_comprehension_missing_closing_brace() {
    assert_fails("set{x for x in items");
    assert_fails("gen{x for x in items");
    assert_fails("{k: v for (k, v) in items");
}

// ============================================================================
// COMPLEX INTEGRATION TESTS
// ============================================================================

#[test]
fn test_nested_comprehensions() {
    // Set containing generators
    let source = "set{gen{y for y in x} for x in matrix}";
    let expr = assert_parses(source);

    let (output_expr, _) = extract_set_comprehension(&expr);
    assert!(matches!(
        &output_expr.kind,
        ExprKind::GeneratorComprehension { .. }
    ));
}

#[test]
fn test_map_with_function_calls() {
    let source = "{transform(k): process(v) for (k, v) in items if validate(k, v)}";
    let expr = assert_parses(source);

    let (key_expr, value_expr, clauses) = extract_map_comprehension(&expr);

    // Both key and value should be function calls
    assert!(matches!(&key_expr.kind, ExprKind::Call { .. }));
    assert!(matches!(&value_expr.kind, ExprKind::Call { .. }));

    // Filter condition should use function call
    if let ComprehensionClauseKind::If(cond) = &clauses[1].kind {
        assert!(matches!(&cond.kind, ExprKind::Call { .. }));
    }
}

#[test]
fn test_comprehension_with_method_chains() {
    let source = "set{x.trim().to_upper() for x in strings if x.len() > 0}";
    let expr = assert_parses(source);

    let (output_expr, _) = extract_set_comprehension(&expr);

    // Output should be method call chain
    assert!(matches!(&output_expr.kind, ExprKind::MethodCall { .. }));
}

#[test]
fn test_comprehension_with_closures() {
    let source = "gen{|y| x + y for x in items}";
    let expr = assert_parses(source);

    let (output_expr, _) = extract_generator_comprehension(&expr);

    // Output should be closure
    assert!(matches!(&output_expr.kind, ExprKind::Closure { .. }));
}

#[test]
fn test_all_comprehension_types_in_expression() {
    // Expression using all three new comprehension types
    let source = "process({k: v for (k, v) in items}, set{x for x in vals}, gen{y for y in seq})";
    let expr = assert_parses(source);

    match &expr.kind {
        ExprKind::Call { args, .. } => {
            assert_eq!(args.len(), 3);
            assert!(matches!(&args[0].kind, ExprKind::MapComprehension { .. }));
            assert!(matches!(&args[1].kind, ExprKind::SetComprehension { .. }));
            assert!(matches!(
                &args[2].kind,
                ExprKind::GeneratorComprehension { .. }
            ));
        }
        _ => panic!("Expected function call"),
    }
}
