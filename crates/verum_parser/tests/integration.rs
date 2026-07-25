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
//! Comprehensive test suite for the Verum parser.
//!
//! This module contains tests for all grammar constructs including:
//! - All statement types (let, return, break, continue, loop, while, for, match)
//! - All expression types (binary ops, unary ops, calls, member access, indexing)
//! - All declaration types (functions, types, protocols, modules)
//! - Error recovery scenarios
//! - Edge cases (nested expressions, complex patterns)

use verum_ast::span::FileId;
use verum_ast::{BinOp, ExprKind, ItemKind, UnOp};
use verum_lexer::Lexer;
use verum_parser::VerumParser;

// ============================================================================
// Basic Parsing Tests
// ============================================================================

#[test]
fn test_parse_empty_source() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);
    let lexer = Lexer::new("", file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
    let module = result.unwrap();
    assert_eq!(module.items.len(), 0);
}

#[test]
fn test_parse_integer_expression() {
    let parser = VerumParser::new();
    let result = parser.parse_expr_str("42", FileId::new(0));
    assert!(result.is_ok());
}

// ============================================================================
// Expression Tests - Literals
// ============================================================================

#[test]
fn test_parse_literal_integer() {
    let parser = VerumParser::new();
    let result = parser.parse_expr_str("12345", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Literal(_)));
}

#[test]
fn test_parse_literal_float() {
    let parser = VerumParser::new();
    let result = parser.parse_expr_str("3.14159", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Literal(_)));
}

#[test]
fn test_parse_literal_string() {
    let parser = VerumParser::new();
    let result = parser.parse_expr_str(r#""hello world""#, FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Literal(_)));
}

#[test]
fn test_parse_literal_bool() {
    let parser = VerumParser::new();

    let result = parser.parse_expr_str("true", FileId::new(0));
    assert!(result.is_ok());

    let result = parser.parse_expr_str("false", FileId::new(0));
    assert!(result.is_ok());
}

// ============================================================================
// Expression Tests - Binary Operators
// ============================================================================

#[test]
fn test_parse_binary_arithmetic() {
    let parser = VerumParser::new();

    // Addition
    let result = parser.parse_expr_str("1 + 2", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Add, .. }));

    // Subtraction
    let result = parser.parse_expr_str("10 - 5", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Sub, .. }));

    // Multiplication
    let result = parser.parse_expr_str("3 * 4", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Mul, .. }));

    // Division
    let result = parser.parse_expr_str("20 / 5", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Div, .. }));

    // Remainder
    let result = parser.parse_expr_str("10 % 3", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Rem, .. }));
}

#[test]
fn test_parse_binary_comparison() {
    let parser = VerumParser::new();

    // Equality
    let result = parser.parse_expr_str("x == y", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Eq, .. }));

    // Inequality
    let result = parser.parse_expr_str("x != y", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Ne, .. }));

    // Less than
    let result = parser.parse_expr_str("x < y", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Lt, .. }));

    // Greater than
    let result = parser.parse_expr_str("x > y", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Gt, .. }));

    // Less than or equal
    let result = parser.parse_expr_str("x <= y", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Le, .. }));

    // Greater than or equal
    let result = parser.parse_expr_str("x >= y", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Ge, .. }));
}

#[test]
fn test_parse_binary_logical() {
    let parser = VerumParser::new();

    // Logical AND
    let result = parser.parse_expr_str("a && b", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::And, .. }));

    // Logical OR
    let result = parser.parse_expr_str("a || b", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Or, .. }));
}

#[test]
fn test_parse_binary_bitwise() {
    let parser = VerumParser::new();

    // Bitwise AND
    let result = parser.parse_expr_str("a & b", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(
        expr.kind,
        ExprKind::Binary {
            op: BinOp::BitAnd,
            ..
        }
    ));

    // Bitwise OR
    let result = parser.parse_expr_str("a | b", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(
        expr.kind,
        ExprKind::Binary {
            op: BinOp::BitOr,
            ..
        }
    ));

    // Bitwise XOR
    let result = parser.parse_expr_str("a ^ b", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(
        expr.kind,
        ExprKind::Binary {
            op: BinOp::BitXor,
            ..
        }
    ));

    // Left shift
    let result = parser.parse_expr_str("a << b", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Shl, .. }));

    // Right shift
    let result = parser.parse_expr_str("a >> b", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Shr, .. }));
}

// ============================================================================
// Expression Tests - Unary Operators
// ============================================================================

#[test]
fn test_parse_unary_operators() {
    let parser = VerumParser::new();

    // Negation
    let result = parser.parse_expr_str("-x", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Unary { op: UnOp::Neg, .. }));

    // Logical NOT
    let result = parser.parse_expr_str("!x", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Unary { op: UnOp::Not, .. }));

    // Bitwise NOT
    let result = parser.parse_expr_str("~x", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(
        expr.kind,
        ExprKind::Unary {
            op: UnOp::BitNot,
            ..
        }
    ));

    // Dereference
    let result = parser.parse_expr_str("*ptr", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(
        expr.kind,
        ExprKind::Unary {
            op: UnOp::Deref,
            ..
        }
    ));
}

// ============================================================================
// Expression Tests - Function Calls and Method Calls
// ============================================================================

#[test]
fn test_parse_function_call() {
    let parser = VerumParser::new();

    // No arguments
    let result = parser.parse_expr_str("foo()", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Call { .. }));

    // Single argument
    let result = parser.parse_expr_str("foo(x)", FileId::new(0));
    assert!(result.is_ok());

    // Multiple arguments
    let result = parser.parse_expr_str("foo(x, y, z)", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    if let ExprKind::Call { args, .. } = expr.kind {
        assert_eq!(args.len(), 3);
    } else {
        panic!("Expected Call expression");
    }
}

#[test]
fn test_parse_method_call() {
    let parser = VerumParser::new();

    // No arguments
    let result = parser.parse_expr_str("obj.method()", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::MethodCall { .. }));

    // With arguments
    let result = parser.parse_expr_str("obj.method(a, b)", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    if let ExprKind::MethodCall { args, .. } = expr.kind {
        assert_eq!(args.len(), 2);
    } else {
        panic!("Expected MethodCall expression");
    }
}

#[test]
fn test_parse_chained_method_calls() {
    let parser = VerumParser::new();
    let result = parser.parse_expr_str("obj.method1().method2().method3()", FileId::new(0));
    assert!(result.is_ok());
}

// ============================================================================
// Expression Tests - Member Access and Indexing
// ============================================================================

#[test]
fn test_parse_field_access() {
    let parser = VerumParser::new();

    let result = parser.parse_expr_str("obj.field", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Field { .. }));
}

#[test]
fn test_parse_tuple_index() {
    let parser = VerumParser::new();

    let result = parser.parse_expr_str("tuple.0", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::TupleIndex { .. }));

    let result = parser.parse_expr_str("tuple.42", FileId::new(0));
    assert!(result.is_ok());
}

#[test]
fn test_parse_array_indexing() {
    let parser = VerumParser::new();

    let result = parser.parse_expr_str("arr[0]", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Index { .. }));

    let result = parser.parse_expr_str("arr[i + 1]", FileId::new(0));
    assert!(result.is_ok());
}

#[test]
fn test_parse_optional_chaining() {
    let parser = VerumParser::new();

    let result = parser.parse_expr_str("obj?.field", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::OptionalChain { .. }));
}

// ============================================================================
// Expression Tests - Special Operators
// ============================================================================

#[test]
fn test_parse_pipeline_operator() {
    let parser = VerumParser::new();

    let result = parser.parse_expr_str("x |> f", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Pipeline { .. }));

    // Chained pipelines
    let result = parser.parse_expr_str("x |> f |> g |> h", FileId::new(0));
    assert!(result.is_ok());
}

#[test]
fn test_parse_null_coalesce() {
    let parser = VerumParser::new();

    let result = parser.parse_expr_str("a ?? b", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::NullCoalesce { .. }));

    // Chained null coalesce
    let result = parser.parse_expr_str("a ?? b ?? c", FileId::new(0));
    assert!(result.is_ok());
}

#[test]
fn test_parse_try_operator() {
    let parser = VerumParser::new();

    let result = parser.parse_expr_str("foo()?", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Try(_)));
}

#[test]
fn test_parse_cast_operator() {
    let parser = VerumParser::new();

    let result = parser.parse_expr_str("x as Int", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Cast { .. }));
}

#[test]
fn test_parse_range_operators() {
    let parser = VerumParser::new();

    // Exclusive range
    let result = parser.parse_expr_str("1..10", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(
        expr.kind,
        ExprKind::Range {
            inclusive: false,
            ..
        }
    ));

    // Inclusive range
    let result = parser.parse_expr_str("1..=10", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(
        expr.kind,
        ExprKind::Range {
            inclusive: true,
            ..
        }
    ));
}

// ============================================================================
// Expression Tests - Collections
// ============================================================================

#[test]
fn test_parse_tuple_expr() {
    let parser = VerumParser::new();

    // Empty tuple
    let result = parser.parse_expr_str("()", FileId::new(0));
    assert!(result.is_ok());

    // Single element (needs trailing comma to disambiguate from grouping)
    let result = parser.parse_expr_str("(1,)", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Tuple(_)));

    // Multiple elements
    let result = parser.parse_expr_str("(1, 2, 3)", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    if let ExprKind::Tuple(elements) = expr.kind {
        assert_eq!(elements.len(), 3);
    } else {
        panic!("Expected Tuple expression");
    }
}

#[test]
fn test_parse_array_expr() {
    let parser = VerumParser::new();

    // Empty array
    let result = parser.parse_expr_str("[]", FileId::new(0));
    assert!(result.is_ok());

    // Array with elements
    let result = parser.parse_expr_str("[1, 2, 3, 4, 5]", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Array(_)));

    // Array repeat syntax
    let result = parser.parse_expr_str("[0; 10]", FileId::new(0));
    assert!(result.is_ok());
}

#[test]
fn test_parse_comprehension() {
    let parser = VerumParser::new();

    // Basic comprehension
    let result = parser.parse_expr_str("[x for x in list]", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Comprehension { .. }));

    // With filter
    let result = parser.parse_expr_str("[x for x in list if x > 0]", FileId::new(0));
    assert!(result.is_ok());

    // With transformation
    let result = parser.parse_expr_str("[x * 2 for x in list if x > 0]", FileId::new(0));
    assert!(result.is_ok());
}

#[test]
fn test_parse_stream_comprehension() {
    let parser = VerumParser::new();

    let result = parser.parse_expr_str("stream [x for x in source]", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::StreamComprehension { .. }));
}

// ============================================================================
// Expression Tests - Control Flow
// ============================================================================

#[test]
fn test_parse_if_expr() {
    let parser = VerumParser::new();

    // If without else
    let result = parser.parse_expr_str("if x > 0 { x }", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::If { .. }));

    // If with else
    let result = parser.parse_expr_str("if x > 0 { x } else { -x }", FileId::new(0));
    assert!(result.is_ok());

    // If-else chain
    let result = parser.parse_expr_str(
        "if x > 0 { x } else if x < 0 { -x } else { 0 }",
        FileId::new(0),
    );
    assert!(result.is_ok());
}

#[test]
fn test_parse_match_expr() {
    let parser = VerumParser::new();

    let source = r#"
        match x {
            0 => "zero",
            1 => "one",
            _ => "many"
        }
    "#;

    let result = parser.parse_expr_str(source, FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    if let ExprKind::Match { arms, .. } = expr.kind {
        assert_eq!(arms.len(), 3);
    } else {
        panic!("Expected Match expression");
    }
}

#[test]
fn test_parse_loop_expr() {
    let parser = VerumParser::new();

    // Infinite loop
    let result = parser.parse_expr_str("loop { break }", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Loop { .. }));

    // Labeled loop
    let result = parser.parse_expr_str("'outer: loop { break 'outer }", FileId::new(0));
    assert!(result.is_ok());
}

#[test]
fn test_parse_while_loop() {
    let parser = VerumParser::new();

    let result = parser.parse_expr_str("while x > 0 { x = x - 1 }", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::While { .. }));

    // Labeled while
    let result = parser.parse_expr_str("'label: while true { break 'label }", FileId::new(0));
    assert!(result.is_ok());
}

#[test]
fn test_parse_for_loop() {
    let parser = VerumParser::new();

    let result = parser.parse_expr_str("for x in list { print(x) }", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::For { .. }));

    // Labeled for
    let result = parser.parse_expr_str("'label: for x in list { continue 'label }", FileId::new(0));
    assert!(result.is_ok());
}

#[test]
fn test_parse_break_continue() {
    let parser = VerumParser::new();

    // Break
    let result = parser.parse_expr_str("break", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Break { .. }));

    // Break with label
    let result = parser.parse_expr_str("break 'label", FileId::new(0));
    assert!(result.is_ok());

    // Break with value
    let result = parser.parse_expr_str("break 42", FileId::new(0));
    assert!(result.is_ok());

    // Continue
    let result = parser.parse_expr_str("continue", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Continue { .. }));

    // Continue with label
    let result = parser.parse_expr_str("continue 'label", FileId::new(0));
    assert!(result.is_ok());
}

#[test]
fn test_parse_return_expr() {
    let parser = VerumParser::new();

    // Return without value
    let result = parser.parse_expr_str("return", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Return(_)));

    // Return with value
    let result = parser.parse_expr_str("return 42", FileId::new(0));
    assert!(result.is_ok());
}

// ============================================================================
// Expression Tests - Blocks and Closures
// ============================================================================

#[test]
fn test_parse_block_expr() {
    let parser = VerumParser::new();

    // Empty block - Note: {} might be parsed as empty map/set, not block
    // A block with a statement is unambiguous
    let result = parser.parse_expr_str("{ let x = 1; x }", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Block(_)));

    // Block with statements
    let result = parser.parse_expr_str("{ let x = 1; x + 1 }", FileId::new(0));
    assert!(result.is_ok());
}

#[test]
fn test_parse_closure() {
    let parser = VerumParser::new();

    // Simple closure
    let result = parser.parse_expr_str("|x| x + 1", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExprKind::Closure { .. }));

    // Multiple parameters
    let result = parser.parse_expr_str("|x, y| x + y", FileId::new(0));
    assert!(result.is_ok());

    // With block body
    let result = parser.parse_expr_str("|x| { let y = x * 2; y + 1 }", FileId::new(0));
    assert!(result.is_ok());
}

// ============================================================================
// Statement Tests
// ============================================================================

#[test]
fn test_parse_let_statement() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    // Simple let binding
    let source = "fn test() { let x = 42; }";
    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());

    // Let with type annotation
    let source = "fn test() { let x: Int = 42; }";
    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());

    // Let with pattern
    let source = "fn test() { let (x, y) = pair; }";
    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

#[test]
fn test_parse_let_else_statement() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = r#"
        fn test() {
            let Some(x) = maybe_value else {
                return
            };
        }
    "#;

    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

#[test]
fn test_parse_expression_statement() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    // With semicolon
    let source = "fn test() { foo(); }";
    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());

    // Without semicolon (tail expression)
    let source = "fn test() -> Int { 42 }";
    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

#[test]
fn test_parse_defer_statement() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = r#"
        fn test() {
            defer cleanup();
        }
    "#;

    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

#[test]
fn test_parse_provide_statement() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = r#"
        fn test() {
            provide Logger = ConsoleLogger.new();
        }
    "#;

    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

// ============================================================================
// Declaration Tests - Functions
// ============================================================================

#[test]
fn test_parse_simple_function() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = r#"
        fn add(x: Int, y: Int) -> Int {
            x + y
        }
    "#;

    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
    let module = result.unwrap();
    assert_eq!(module.items.len(), 1);
    assert!(matches!(module.items[0].kind, ItemKind::Function(_)));
}

#[test]
fn test_parse_function_with_generics() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = r#"
        fn identity<T>(x: T) -> T {
            x
        }
    "#;

    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

#[test]
fn test_parse_async_function() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = r#"
        async fn fetch_data() -> Text {
            "data"
        }
    "#;

    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

#[test]
fn test_parse_public_function() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = r#"
        pub fn public_function() -> Int {
            42
        }
    "#;

    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

// ============================================================================
// Declaration Tests - Types
// ============================================================================

#[test]
fn test_parse_type_alias() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = "type UserId is Int;";
    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
    let module = result.unwrap();
    assert_eq!(module.items.len(), 1);
    assert!(matches!(module.items[0].kind, ItemKind::Type(_)));
}

#[test]
fn test_parse_record_type() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = r#"
        type Point is {
            x: Float,
            y: Float
        };
    "#;

    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

#[test]
fn test_parse_variant_type() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = r#"
        type Maybe<T> is
            | Some(T)
            | None;
    "#;

    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

#[test]
fn test_parse_newtype() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    // Note: Verum uses "is" for type definitions
    let source = "type Email is Text;";
    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

// ============================================================================
// Declaration Tests - Protocols and Implementations
// ============================================================================

#[test]
fn test_parse_protocol() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = r#"
        protocol Show {
            fn show(self) -> Text;
        }
    "#;

    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
    let module = result.unwrap();
    assert_eq!(module.items.len(), 1);
    assert!(matches!(module.items[0].kind, ItemKind::Protocol(_)));
}

#[test]
fn test_parse_implementation() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = r#"
        implement Show for Int {
            fn show(self) -> Text {
                int_to_string(self)
            }
        }
    "#;

    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
    let module = result.unwrap();
    assert_eq!(module.items.len(), 1);
    assert!(matches!(module.items[0].kind, ItemKind::Impl(_)));
}

// ============================================================================
// Declaration Tests - Other Items
// ============================================================================

#[test]
fn test_parse_const() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = "const PI: Float = 3.14159;";
    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
    let module = result.unwrap();
    assert_eq!(module.items.len(), 1);
    assert!(matches!(module.items[0].kind, ItemKind::Const(_)));
}

#[test]
fn test_parse_mount() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    // Simple mount
    let source = "mount std.io.print;";
    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());

    // Mount with alias
    let source = "mount std.io.print as println;";
    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

#[test]
fn test_parse_module() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = r#"
        module math {
            fn add(x: Int, y: Int) -> Int {
                x + y
            }
        }
    "#;

    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
    let module = result.unwrap();
    assert_eq!(module.items.len(), 1);
    assert!(matches!(module.items[0].kind, ItemKind::Module(_)));
}

// ============================================================================
// Pattern Tests
// ============================================================================

#[test]
fn test_parse_wildcard_pattern() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = "fn test() { let _ = 42; }";
    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

#[test]
fn test_parse_identifier_pattern() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = "fn test() { let x = 42; }";
    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

#[test]
fn test_parse_tuple_pattern() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = "fn test() { let (x, y, z) = triple; }";
    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

#[test]
fn test_parse_record_pattern() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = "fn test() { let Point { x, y } = point; }";
    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

#[test]
fn test_parse_variant_pattern() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = r#"
        fn test() {
            match opt {
                Some(x) => x,
                None => 0
            }
        }
    "#;

    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

// ============================================================================
// Operator Precedence Tests
// ============================================================================

#[test]
fn test_operator_precedence_arithmetic() {
    let parser = VerumParser::new();

    // Multiplication before addition
    let result = parser.parse_expr_str("1 + 2 * 3", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    // Should parse as 1 + (2 * 3), not (1 + 2) * 3
    if let ExprKind::Binary {
        op: BinOp::Add,
        right,
        ..
    } = expr.kind
    {
        assert!(matches!(
            right.kind,
            ExprKind::Binary { op: BinOp::Mul, .. }
        ));
    } else {
        panic!("Expected addition as top-level operator");
    }
}

#[test]
fn test_operator_precedence_comparison() {
    let parser = VerumParser::new();

    // Arithmetic before comparison
    let result = parser.parse_expr_str("x + 1 < y * 2", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    // Should parse as (x + 1) < (y * 2)
    assert!(matches!(expr.kind, ExprKind::Binary { op: BinOp::Lt, .. }));
}

#[test]
fn test_operator_precedence_logical() {
    let parser = VerumParser::new();

    // AND before OR
    let result = parser.parse_expr_str("a || b && c", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    // Should parse as a || (b && c), not (a || b) && c
    if let ExprKind::Binary {
        op: BinOp::Or,
        right,
        ..
    } = expr.kind
    {
        assert!(matches!(
            right.kind,
            ExprKind::Binary { op: BinOp::And, .. }
        ));
    } else {
        panic!("Expected OR as top-level operator");
    }
}

#[test]
fn test_operator_associativity() {
    let parser = VerumParser::new();

    // Left associativity for subtraction
    let result = parser.parse_expr_str("10 - 5 - 2", FileId::new(0));
    assert!(result.is_ok());
    let expr = result.unwrap();
    // Should parse as (10 - 5) - 2, not 10 - (5 - 2)
    if let ExprKind::Binary {
        op: BinOp::Sub,
        left,
        ..
    } = expr.kind
    {
        assert!(matches!(left.kind, ExprKind::Binary { op: BinOp::Sub, .. }));
    } else {
        panic!("Expected subtraction as top-level operator");
    }
}

// ============================================================================
// Complex Expression Tests
// ============================================================================

#[test]
fn test_nested_expressions() {
    let parser = VerumParser::new();

    // Deeply nested arithmetic
    let result = parser.parse_expr_str("((1 + 2) * (3 - 4)) / 5", FileId::new(0));
    assert!(result.is_ok());

    // Nested function calls
    let result = parser.parse_expr_str("f(g(h(x)))", FileId::new(0));
    assert!(result.is_ok());

    // Nested method calls with indexing
    let result = parser.parse_expr_str("obj.method()[0].field", FileId::new(0));
    assert!(result.is_ok());
}

#[test]
fn test_complex_match_expression() {
    let parser = VerumParser::new();

    let source = r#"
        match result {
            Ok(value) if value > 0 => value * 2,
            Ok(value) => value,
            Err(msg) => {
                log(msg);
                0
            }
        }
    "#;

    let result = parser.parse_expr_str(source, FileId::new(0));
    assert!(result.is_ok());
}

#[test]
fn test_nested_control_flow() {
    let parser = VerumParser::new();

    let source = r#"
        for i in 0..10 {
            if i % 2 == 0 {
                while j < i {
                    j = j + 1
                }
            } else {
                continue
            }
        }
    "#;

    let result = parser.parse_expr_str(source, FileId::new(0));
    assert!(result.is_ok());
}

// ============================================================================
// Error Recovery Tests
// ============================================================================

#[test]
fn test_error_recovery_missing_semicolon() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    // VCS E010: Missing semicolon after let statement should be an error
    let source = r#"
        fn test() {
            let x = 1
            let y = 2;
        }
    "#;

    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    // Should detect missing semicolon error (VCS E010)
    assert!(result.is_err());
}

#[test]
fn test_error_recovery_missing_brace() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = r#"
        fn test() {
            let x = 1;
    "#;

    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    // Should detect the error
    assert!(result.is_err());
}

#[test]
fn test_error_recovery_invalid_expression() {
    let parser = VerumParser::new();

    // Invalid operator sequence
    let result = parser.parse_expr_str("1 ++ 2", FileId::new(0));
    // Parser should detect this as an error
    // Note: `++` is not a valid Verum operator
    assert!(result.is_err());
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_empty_function_body() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = "fn empty() {}";
    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

#[test]
fn test_trailing_comma_in_call() {
    let parser = VerumParser::new();

    let result = parser.parse_expr_str("foo(a, b, c,)", FileId::new(0));
    assert!(result.is_ok());
}

#[test]
fn test_trailing_comma_in_tuple() {
    let parser = VerumParser::new();

    let result = parser.parse_expr_str("(1, 2, 3,)", FileId::new(0));
    assert!(result.is_ok());
}

#[test]
fn test_unicode_identifiers() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = "fn привет() -> Int { 42 }";
    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

#[test]
fn test_multiline_expressions() {
    let parser = VerumParser::new();

    let source = r#"
        1 +
        2 +
        3
    "#;

    let result = parser.parse_expr_str(source, FileId::new(0));
    assert!(result.is_ok());
}

#[test]
fn test_comments_in_expressions() {
    let parser = VerumParser::new();

    let source = r#"
        1 + /* comment */ 2
    "#;

    let result = parser.parse_expr_str(source, FileId::new(0));
    assert!(result.is_ok());
}

// ============================================================================
// Refinement Type Tests
// ============================================================================

#[test]
fn test_parse_simple_refinement_type() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = "type Positive is Int{> 0};";
    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

#[test]
fn test_parse_refinement_type_with_function() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = "fn abs(x: Int) -> Int{>= 0} { if x < 0 { -x } else { x } }";
    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
}

// ============================================================================
// Integration Tests - Complete Modules
// ============================================================================

#[test]
fn test_parse_complete_module() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    let source = r#"
        mount std.io.print;

        const MAX_SIZE: Int = 100;

        type Point is {
            x: Float,
            y: Float
        };

        fn distance(p1: Point, p2: Point) -> Float {
            let dx = p2.x - p1.x;
            let dy = p2.y - p1.y;
            sqrt(dx * dx + dy * dy)
        }

        fn main() {
            let origin = Point { x: 0.0, y: 0.0 };
            let point = Point { x: 3.0, y: 4.0 };
            let dist = distance(origin, point);
            print(f"Distance: {dist}");
        }
    "#;

    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
    let module = result.unwrap();
    assert!(module.items.len() >= 4); // mount, const, type, functions
}
