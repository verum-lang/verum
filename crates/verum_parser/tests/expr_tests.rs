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
// Tests for expression parsing
//
// Tests for expression parsing: literals, operators, paths, closures, blocks, etc.
// This module tests parsing of all Verum expression forms including:
// - Literals (integers, floats, strings, chars, booleans)
// - Binary operations with proper precedence
// - Unary operations
// - Function calls and method calls
// - Field access and indexing
// - Control flow (if, match, loops)
// - Comprehensions and pipelines

use verum_ast::{BinOp, Expr, ExprKind, FileId, UnOp};
use verum_lexer::Lexer;
use verum_parser::VerumParser;

fn parse_expr_test(source: &str) -> Expr {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser
        .parse_expr_str(source, file_id)
        .unwrap_or_else(|_| panic!("Failed to parse: {}", source))
}

// === LITERAL TESTS ===

#[test]
fn test_parse_integer_literal() {
    let expr = parse_expr_test("42");
    match &expr.kind {
        ExprKind::Literal(lit) => {
            assert_eq!(lit.span, expr.span, "Literal spans should match");
        }
        _ => panic!("Expected literal expression"),
    }
}

#[test]
fn test_parse_float_literal() {
    let expr = parse_expr_test("3.14");
    match &expr.kind {
        ExprKind::Literal(lit) => {
            assert_eq!(lit.span, expr.span, "Literal spans should match");
        }
        _ => panic!("Expected literal expression"),
    }
}

#[test]
fn test_parse_string_literal() {
    let expr = parse_expr_test(r#""hello world""#);
    match &expr.kind {
        ExprKind::Literal(_lit) => {
            // String parsing is correct
        }
        _ => panic!("Expected literal expression"),
    }
}

#[test]
fn test_parse_char_literal() {
    // Note: Single unescaped chars like 'x' are now lifetimes, not char literals
    // Char literals must use escape sequences like '\x'
    let expr = parse_expr_test(r"'\n'");
    match &expr.kind {
        ExprKind::Literal(_lit) => {
            // Char parsing is correct
        }
        _ => panic!("Expected literal expression"),
    }
}

#[test]
fn test_parse_bool_true() {
    let expr = parse_expr_test("true");
    match &expr.kind {
        ExprKind::Literal(_lit) => {
            // Bool parsing is correct
        }
        _ => panic!("Expected literal expression"),
    }
}

#[test]
fn test_parse_bool_false() {
    let expr = parse_expr_test("false");
    match &expr.kind {
        ExprKind::Literal(_lit) => {
            // Bool parsing is correct
        }
        _ => panic!("Expected literal expression"),
    }
}

// === BINARY OPERATION TESTS ===

#[test]
fn test_parse_addition() {
    let expr = parse_expr_test("1 + 2");
    match &expr.kind {
        ExprKind::Binary { op, .. } => {
            assert_eq!(*op, BinOp::Add, "Expected addition operator");
        }
        _ => panic!("Expected binary operation"),
    }
}

#[test]
fn test_parse_subtraction() {
    let expr = parse_expr_test("10 - 3");
    match &expr.kind {
        ExprKind::Binary { op, .. } => {
            assert_eq!(*op, BinOp::Sub, "Expected subtraction operator");
        }
        _ => panic!("Expected binary operation"),
    }
}

#[test]
fn test_parse_multiplication() {
    let expr = parse_expr_test("5 * 6");
    match &expr.kind {
        ExprKind::Binary { op, .. } => {
            assert_eq!(*op, BinOp::Mul, "Expected multiplication operator");
        }
        _ => panic!("Expected binary operation"),
    }
}

#[test]
fn test_parse_division() {
    let expr = parse_expr_test("20 / 4");
    match &expr.kind {
        ExprKind::Binary { op, .. } => {
            assert_eq!(*op, BinOp::Div, "Expected division operator");
        }
        _ => panic!("Expected binary operation"),
    }
}

#[test]
fn test_parse_remainder() {
    let expr = parse_expr_test("10 % 3");
    match &expr.kind {
        ExprKind::Binary { op, .. } => {
            assert_eq!(*op, BinOp::Rem, "Expected remainder operator");
        }
        _ => panic!("Expected binary operation"),
    }
}

#[test]
fn test_parse_power() {
    let expr = parse_expr_test("2 ** 3");
    match &expr.kind {
        ExprKind::Binary { op, .. } => {
            assert_eq!(*op, BinOp::Pow, "Expected power operator");
        }
        _ => panic!("Expected binary operation"),
    }
}

#[test]
fn test_parse_equality() {
    let expr = parse_expr_test("5 == 5");
    match &expr.kind {
        ExprKind::Binary { op, .. } => {
            assert_eq!(*op, BinOp::Eq, "Expected equality operator");
        }
        _ => panic!("Expected binary operation"),
    }
}

#[test]
fn test_parse_not_equal() {
    let expr = parse_expr_test("5 != 3");
    match &expr.kind {
        ExprKind::Binary { op, .. } => {
            assert_eq!(*op, BinOp::Ne, "Expected not-equal operator");
        }
        _ => panic!("Expected binary operation"),
    }
}

#[test]
fn test_parse_less_than() {
    let expr = parse_expr_test("3 < 5");
    match &expr.kind {
        ExprKind::Binary { op, .. } => {
            assert_eq!(*op, BinOp::Lt, "Expected less-than operator");
        }
        _ => panic!("Expected binary operation"),
    }
}

#[test]
fn test_parse_greater_than() {
    let expr = parse_expr_test("5 > 3");
    match &expr.kind {
        ExprKind::Binary { op, .. } => {
            assert_eq!(*op, BinOp::Gt, "Expected greater-than operator");
        }
        _ => panic!("Expected binary operation"),
    }
}

#[test]
fn test_parse_logical_and() {
    let expr = parse_expr_test("true && false");
    match &expr.kind {
        ExprKind::Binary { op, .. } => {
            assert_eq!(*op, BinOp::And, "Expected logical AND operator");
        }
        _ => panic!("Expected binary operation"),
    }
}

#[test]
fn test_parse_logical_or() {
    let expr = parse_expr_test("true || false");
    match &expr.kind {
        ExprKind::Binary { op, .. } => {
            assert_eq!(*op, BinOp::Or, "Expected logical OR operator");
        }
        _ => panic!("Expected binary operation"),
    }
}

#[test]
fn test_parse_bitwise_and() {
    let expr = parse_expr_test("5 & 3");
    match &expr.kind {
        ExprKind::Binary { op, .. } => {
            assert_eq!(*op, BinOp::BitAnd, "Expected bitwise AND operator");
        }
        _ => panic!("Expected binary operation"),
    }
}

#[test]
fn test_parse_bitwise_or() {
    let expr = parse_expr_test("5 | 3");
    match &expr.kind {
        ExprKind::Binary { op, .. } => {
            assert_eq!(*op, BinOp::BitOr, "Expected bitwise OR operator");
        }
        _ => panic!("Expected binary operation"),
    }
}

#[test]
fn test_parse_bitwise_xor() {
    let expr = parse_expr_test("5 ^ 3");
    match &expr.kind {
        ExprKind::Binary { op, .. } => {
            assert_eq!(*op, BinOp::BitXor, "Expected bitwise XOR operator");
        }
        _ => panic!("Expected binary operation"),
    }
}

#[test]
fn test_parse_left_shift() {
    let expr = parse_expr_test("4 << 2");
    match &expr.kind {
        ExprKind::Binary { op, .. } => {
            assert_eq!(*op, BinOp::Shl, "Expected left shift operator");
        }
        _ => panic!("Expected binary operation"),
    }
}

#[test]
fn test_parse_right_shift() {
    let expr = parse_expr_test("16 >> 2");
    match &expr.kind {
        ExprKind::Binary { op, .. } => {
            assert_eq!(*op, BinOp::Shr, "Expected right shift operator");
        }
        _ => panic!("Expected binary operation"),
    }
}

// === OPERATOR PRECEDENCE TESTS ===

#[test]
fn test_parse_precedence_mult_add() {
    // 2 + 3 * 4 should parse as 2 + (3 * 4)
    let expr = parse_expr_test("2 + 3 * 4");
    match &expr.kind {
        ExprKind::Binary {
            op: BinOp::Add,
            left,
            right,
        } => {
            match &right.kind {
                ExprKind::Binary { op: BinOp::Mul, .. } => {
                    // Correct precedence
                }
                _ => panic!("Expected multiplication on right side of addition"),
            }
        }
        _ => panic!("Expected binary operation at top level"),
    }
}

#[test]
fn test_parse_precedence_power() {
    // 2 * 3 ** 2 should parse as 2 * (3 ** 2)
    let expr = parse_expr_test("2 * 3 ** 2");
    match &expr.kind {
        ExprKind::Binary {
            op: BinOp::Mul,
            left: _left,
            right,
        } => {
            match &right.kind {
                ExprKind::Binary { op: BinOp::Pow, .. } => {
                    // Correct precedence
                }
                _ => panic!("Expected power on right side of multiplication"),
            }
        }
        _ => panic!("Expected binary operation at top level"),
    }
}

// === UNARY OPERATION TESTS ===

#[test]
fn test_parse_negation() {
    let expr = parse_expr_test("-5");
    match &expr.kind {
        ExprKind::Unary { op, .. } => {
            assert_eq!(*op, UnOp::Neg, "Expected negation operator");
        }
        _ => panic!("Expected unary operation"),
    }
}

#[test]
fn test_parse_logical_not() {
    let expr = parse_expr_test("!true");
    match &expr.kind {
        ExprKind::Unary { op, .. } => {
            assert_eq!(*op, UnOp::Not, "Expected logical NOT operator");
        }
        _ => panic!("Expected unary operation"),
    }
}

#[test]
fn test_parse_bitwise_not() {
    let expr = parse_expr_test("~5");
    match &expr.kind {
        ExprKind::Unary { op, .. } => {
            assert_eq!(*op, UnOp::BitNot, "Expected bitwise NOT operator");
        }
        _ => panic!("Expected unary operation"),
    }
}

#[test]
fn test_parse_reference() {
    let expr = parse_expr_test("&x");
    match &expr.kind {
        ExprKind::Unary { op, .. } => {
            assert_eq!(*op, UnOp::Ref, "Expected reference operator");
        }
        _ => panic!("Expected unary operation"),
    }
}

#[test]
fn test_parse_dereference() {
    let expr = parse_expr_test("*p");
    match &expr.kind {
        ExprKind::Unary { op, .. } => {
            assert_eq!(*op, UnOp::Deref, "Expected dereference operator");
        }
        _ => panic!("Expected unary operation"),
    }
}

// === FUNCTION CALL TESTS ===

#[test]
fn test_parse_function_call_no_args() {
    let expr = parse_expr_test("foo()");
    match &expr.kind {
        ExprKind::Call { func, type_args: _, args } => {
            assert_eq!(args.len(), 0, "Expected no arguments");
        }
        _ => panic!("Expected function call"),
    }
}

#[test]
fn test_parse_function_call_one_arg() {
    let expr = parse_expr_test("foo(5)");
    match &expr.kind {
        ExprKind::Call { func, type_args: _, args } => {
            assert_eq!(args.len(), 1, "Expected one argument");
        }
        _ => panic!("Expected function call"),
    }
}

#[test]
fn test_parse_function_call_multiple_args() {
    let expr = parse_expr_test("foo(1, 2, 3)");
    match &expr.kind {
        ExprKind::Call { func, type_args: _, args } => {
            assert_eq!(args.len(), 3, "Expected three arguments");
        }
        _ => panic!("Expected function call"),
    }
}

// === FIELD ACCESS TESTS ===

#[test]
fn test_parse_field_access() {
    let expr = parse_expr_test("obj.field");
    match &expr.kind {
        ExprKind::Field { expr: _expr, field } => {
            assert_eq!(field.name.as_str(), "field", "Expected field name");
        }
        _ => panic!("Expected field access, got: {:?}", expr.kind),
    }
}

#[test]
fn test_parse_method_call() {
    let expr = parse_expr_test("obj.method(1, 2)");
    match &expr.kind {
        ExprKind::MethodCall {
            receiver,
            method,
            type_args: _,
            args,
        } => {
            assert_eq!(method.name.as_str(), "method", "Expected method name");
            assert_eq!(args.len(), 2, "Expected two arguments");
        }
        _ => panic!("Expected method call, got: {:?}", expr.kind),
    }
}

#[test]
fn test_parse_chained_field_access() {
    let expr = parse_expr_test("obj.field1.field2");
    match &expr.kind {
        ExprKind::Field { .. } => {
            // Correct structure
        }
        _ => panic!("Expected field access"),
    }
}

// === INDEXING TESTS ===

#[test]
fn test_parse_array_index() {
    let expr = parse_expr_test("arr[0]");
    match &expr.kind {
        ExprKind::Index { expr: _expr, index } => {
            // Index expression should exist
        }
        _ => panic!("Expected array indexing"),
    }
}

#[test]
fn test_parse_nested_index() {
    let expr = parse_expr_test("arr[i][j]");
    match &expr.kind {
        ExprKind::Index { .. } => {
            // Correct structure
        }
        _ => panic!("Expected array indexing"),
    }
}

// === CONTROL FLOW TESTS ===

#[test]
fn test_parse_if_expression() {
    let expr = parse_expr_test("if true { 1 } else { 2 }");
    match &expr.kind {
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            assert!(else_branch.is_some(), "Expected else branch");
        }
        _ => panic!("Expected if expression"),
    }
}

#[test]
fn test_parse_if_without_else() {
    let expr = parse_expr_test("if true { 1 }");
    match &expr.kind {
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            assert!(else_branch.is_none(), "Expected no else branch");
        }
        _ => panic!("Expected if expression"),
    }
}

#[test]
fn test_parse_match_expression() {
    let expr = parse_expr_test("match x { 1 => 2, _ => 3 }");
    match &expr.kind {
        ExprKind::Match { expr: _expr, arms } => {
            assert_eq!(arms.len(), 2, "Expected two match arms");
        }
        _ => panic!("Expected match expression"),
    }
}

#[test]
fn test_parse_loop_expression() {
    let expr = parse_expr_test("loop { break }");
    match &expr.kind {
        ExprKind::Loop {
            label: _,
            body: _,
            invariants: _,
        } => {
            // Correct structure
        }
        _ => panic!("Expected loop expression"),
    }
}

#[test]
fn test_parse_while_expression() {
    let expr = parse_expr_test("while x < 5 { x = x + 1 }");
    match &expr.kind {
        ExprKind::While {
            label: _,
            condition: _,
            body: _,
            invariants: _,
            decreases: _,
        } => {
            // Correct structure
        }
        _ => panic!("Expected while expression"),
    }
}

#[test]
fn test_parse_for_expression() {
    let expr = parse_expr_test("for x in items { }");
    match &expr.kind {
        ExprKind::For {
            label: _,
            pattern: _,
            iter: _,
            body: _,
            invariants: _,
            decreases: _,
        } => {
            // Correct structure
        }
        _ => panic!("Expected for expression"),
    }
}

// === COMPREHENSION TESTS ===

#[test]
fn test_parse_list_comprehension() {
    let expr = parse_expr_test("[x * 2 for x in items]");
    match &expr.kind {
        ExprKind::Comprehension {
            expr: _expr,
            clauses,
        } => {
            assert!(!clauses.is_empty(), "Expected at least one clause");
        }
        _ => panic!("Expected comprehension expression"),
    }
}

#[test]
fn test_parse_stream_comprehension() {
    let expr = parse_expr_test("stream [x * 2 for x in items]");
    match &expr.kind {
        ExprKind::StreamComprehension {
            expr: _expr,
            clauses,
        } => {
            assert!(!clauses.is_empty(), "Expected at least one clause");
        }
        _ => panic!("Expected stream comprehension expression"),
    }
}

#[test]
fn test_parse_comprehension_with_filter() {
    let expr = parse_expr_test("[x for x in items if x > 0]");
    match &expr.kind {
        ExprKind::Comprehension {
            expr: _expr,
            clauses,
        } => {
            assert!(clauses.len() >= 2, "Expected for clause and filter clause");
        }
        _ => panic!("Expected comprehension expression"),
    }
}

// === PIPELINE TESTS ===

#[test]
fn test_parse_pipeline_operator() {
    let expr = parse_expr_test("x |> f");
    match &expr.kind {
        ExprKind::Pipeline { left, right } => {
            // Correct structure
        }
        _ => panic!("Expected pipeline expression"),
    }
}

#[test]
fn test_parse_pipeline_chain() {
    let expr = parse_expr_test("x |> f |> g |> h");
    match &expr.kind {
        ExprKind::Pipeline { .. } => {
            // Correct structure
        }
        _ => panic!("Expected pipeline expression"),
    }
}

// === PARENTHESIZED EXPRESSION TESTS ===

#[test]
fn test_parse_parenthesized_expression() {
    let expr = parse_expr_test("(1 + 2)");
    match &expr.kind {
        ExprKind::Paren(inner) => {
            // Correct structure
        }
        _ => panic!("Expected parenthesized expression"),
    }
}

#[test]
fn test_parse_tuple_expression() {
    let expr = parse_expr_test("(1, 2, 3)");
    match &expr.kind {
        ExprKind::Tuple(elements) => {
            assert_eq!(elements.len(), 3, "Expected three elements");
        }
        _ => panic!("Expected tuple expression"),
    }
}

#[test]
fn test_parse_tuple_with_two_elements() {
    let expr = parse_expr_test("(1, 2)");
    match &expr.kind {
        ExprKind::Tuple(elements) => {
            assert_eq!(elements.len(), 2, "Expected two elements");
        }
        _ => panic!("Expected tuple expression"),
    }
}

// === ARRAY TESTS ===

#[test]
fn test_parse_array_literal() {
    let expr = parse_expr_test("[1, 2, 3]");
    match &expr.kind {
        ExprKind::Array(_array) => {
            // Correct structure
        }
        _ => panic!("Expected array expression"),
    }
}

#[test]
fn test_parse_array_repeat() {
    let expr = parse_expr_test("[1; 5]");
    match &expr.kind {
        ExprKind::Array(_array) => {
            // Correct structure
        }
        _ => panic!("Expected array repeat expression"),
    }
}

// === CLOSURE TESTS ===

#[test]
fn test_parse_closure_expression() {
    let expr = parse_expr_test("|x| x + 1");
    match &expr.kind {
        ExprKind::Closure { params, body, .. } => {
            assert_eq!(params.len(), 1, "Expected one parameter");
        }
        _ => panic!("Expected closure expression"),
    }
}

#[test]
fn test_parse_closure_multiple_params() {
    let expr = parse_expr_test("|x, y| x + y");
    match &expr.kind {
        ExprKind::Closure { params, body, .. } => {
            assert_eq!(params.len(), 2, "Expected two parameters");
        }
        _ => panic!("Expected closure expression"),
    }
}

// === BLOCK EXPRESSION TESTS ===

#[test]
fn test_parse_block_expression() {
    let expr = parse_expr_test("{ let x = 5; x + 1 }");
    match &expr.kind {
        ExprKind::Block(block) => {
            // Correct structure
        }
        _ => panic!("Expected block expression"),
    }
}

// === BREAK/CONTINUE/RETURN TESTS ===

#[test]
fn test_parse_break_without_value() {
    let expr = parse_expr_test("break");
    match &expr.kind {
        ExprKind::Break { label: _, value } => {
            assert!(value.is_none(), "Expected no break value");
        }
        _ => panic!("Expected break expression"),
    }
}

#[test]
fn test_parse_break_with_value() {
    let expr = parse_expr_test("break 42");
    match &expr.kind {
        ExprKind::Break { label: _, value } => {
            assert!(value.is_some(), "Expected break value");
        }
        _ => panic!("Expected break expression"),
    }
}

#[test]
fn test_parse_continue() {
    let expr = parse_expr_test("continue");
    match &expr.kind {
        ExprKind::Continue { .. } => {
            // Correct structure
        }
        _ => panic!("Expected continue expression"),
    }
}

#[test]
fn test_parse_return_without_value() {
    let expr = parse_expr_test("return");
    match &expr.kind {
        ExprKind::Return(value) => {
            assert!(value.is_none(), "Expected no return value");
        }
        _ => panic!("Expected return expression"),
    }
}

#[test]
fn test_parse_return_with_value() {
    let expr = parse_expr_test("return 42");
    match &expr.kind {
        ExprKind::Return(value) => {
            assert!(value.is_some(), "Expected return value");
        }
        _ => panic!("Expected return expression"),
    }
}

// === TRY EXPRESSION TESTS ===

#[test]
fn test_parse_try_recover() {
    let expr = parse_expr_test("try { } recover { }");
    match &expr.kind {
        ExprKind::TryRecover {
            try_block,
            recover,
        } => {
            // Correct structure
        }
        _ => panic!("Expected try-recover expression"),
    }
}

#[test]
fn test_parse_try_finally() {
    let expr = parse_expr_test("try { } finally { }");
    match &expr.kind {
        ExprKind::TryFinally {
            try_block,
            finally_block,
        } => {
            // Correct structure
        }
        _ => panic!("Expected try-finally expression"),
    }
}

// === NULL COALESCING TESTS ===

#[test]
fn test_parse_null_coalescing() {
    let expr = parse_expr_test("a ?? b");
    match &expr.kind {
        ExprKind::NullCoalesce { left, right } => {
            // Correct structure
        }
        _ => panic!("Expected null coalescing expression"),
    }
}

#[test]
fn test_parse_null_coalescing_chain() {
    let expr = parse_expr_test("a ?? b ?? c");
    match &expr.kind {
        ExprKind::NullCoalesce { .. } => {
            // Correct structure
        }
        _ => panic!("Expected null coalescing expression"),
    }
}

// === COMPLEX EXPRESSION TESTS ===

#[test]
fn test_parse_nested_function_calls() {
    let expr = parse_expr_test("f(g(h(x)))");
    match &expr.kind {
        ExprKind::Call { .. } => {
            // Should be properly nested
        }
        _ => panic!("Expected nested function calls"),
    }
}

#[test]
fn test_parse_mixed_operators() {
    let expr = parse_expr_test("a + b * c - d / e");
    match &expr.kind {
        ExprKind::Binary { .. } => {
            // Should respect operator precedence
        }
        _ => panic!("Expected binary operation"),
    }
}

#[test]
fn test_parse_try_recover_with_arms() {
    let expr = parse_expr_test("try { 50 } recover { SomeError(msg) => -1, _ => 0 }");
    match &expr.kind {
        ExprKind::TryRecover { recover, .. } => {
            if let verum_ast::expr::RecoverBody::MatchArms { arms, .. } = recover {
                assert_eq!(arms.len(), 2, "Expected two match arms");
            } else {
                panic!("Expected MatchArms recover body");
            }
        }
        _ => panic!("Expected TryRecover expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_parse_try_recover_single_arm() {
    let expr = parse_expr_test("try { 50 } recover { _ => -1 }");
    match &expr.kind {
        ExprKind::TryRecover { recover, .. } => {
            if let verum_ast::expr::RecoverBody::MatchArms { arms, .. } = recover {
                assert_eq!(arms.len(), 1, "Expected one match arm");
            } else {
                panic!("Expected MatchArms recover body");
            }
        }
        _ => panic!("Expected TryRecover expression, got {:?}", expr.kind),
    }
}

// === RECOVER CLOSURE SYNTAX TESTS ===
// Grammar: recover_closure = closure_params , recover_closure_body ;
//          recover_closure_body = block_expr | expression ;

#[test]
fn test_parse_try_recover_closure_with_block() {
    // recover |e| { handle_error(e) }
    let expr = parse_expr_test("try { risky() } recover |e| { log(e) }");
    match &expr.kind {
        ExprKind::TryRecover { recover, .. } => {
            if let verum_ast::expr::RecoverBody::Closure { param, body, .. } = recover {
                // Param should be an identifier pattern
                assert!(
                    matches!(param.pattern.kind, verum_ast::pattern::PatternKind::Ident { .. }),
                    "Expected ident pattern for closure param"
                );
                // Body should be a block expression
                assert!(
                    matches!(body.kind, ExprKind::Block(_)),
                    "Expected block body"
                );
            } else {
                panic!("Expected Closure recover body");
            }
        }
        _ => panic!("Expected TryRecover expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_parse_try_recover_closure_with_expression() {
    // recover |e| log_error(e) (expression without block)
    let expr = parse_expr_test("try { risky() } recover |e| log_error(e)");
    match &expr.kind {
        ExprKind::TryRecover { recover, .. } => {
            if let verum_ast::expr::RecoverBody::Closure { param, body, .. } = recover {
                // Param should be an identifier pattern
                assert!(
                    matches!(param.pattern.kind, verum_ast::pattern::PatternKind::Ident { .. }),
                    "Expected ident pattern for closure param"
                );
                // Body should be a call expression (not a block)
                assert!(
                    matches!(body.kind, ExprKind::Call { .. }),
                    "Expected call expression body, got {:?}",
                    body.kind
                );
            } else {
                panic!("Expected Closure recover body");
            }
        }
        _ => panic!("Expected TryRecover expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_parse_try_recover_closure_with_wildcard() {
    // recover |_| default_value
    let expr = parse_expr_test("try { risky() } recover |_| default_value");
    match &expr.kind {
        ExprKind::TryRecover { recover, .. } => {
            if let verum_ast::expr::RecoverBody::Closure { param, body, .. } = recover {
                // Param should be wildcard pattern
                assert!(
                    matches!(param.pattern.kind, verum_ast::pattern::PatternKind::Wildcard),
                    "Expected wildcard pattern for closure param"
                );
            } else {
                panic!("Expected Closure recover body");
            }
        }
        _ => panic!("Expected TryRecover expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_parse_try_recover_closure_with_typed_param() {
    // recover |e: Error| handle(e)
    let expr = parse_expr_test("try { risky() } recover |e: Error| handle(e)");
    match &expr.kind {
        ExprKind::TryRecover { recover, .. } => {
            if let verum_ast::expr::RecoverBody::Closure { param, body, .. } = recover {
                // Param should have type annotation
                assert!(
                    param.ty.is_some(),
                    "Expected type annotation on closure param"
                );
            } else {
                panic!("Expected Closure recover body");
            }
        }
        _ => panic!("Expected TryRecover expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_parse_try_recover_closure_with_finally() {
    // try { ... } recover |e| handle(e) finally { cleanup() }
    let expr = parse_expr_test("try { risky() } recover |e| handle(e) finally { cleanup() }");
    match &expr.kind {
        ExprKind::TryRecoverFinally { recover, finally_block, .. } => {
            if let verum_ast::expr::RecoverBody::Closure { param, body, .. } = recover {
                // Param should be an identifier pattern
                assert!(
                    matches!(param.pattern.kind, verum_ast::pattern::PatternKind::Ident { .. }),
                    "Expected ident pattern for closure param"
                );
            } else {
                panic!("Expected Closure recover body");
            }
            // Finally block should be present
            assert!(
                matches!(finally_block.kind, ExprKind::Block(_)),
                "Expected block for finally"
            );
        }
        _ => panic!("Expected TryRecoverFinally expression, got {:?}", expr.kind),
    }
}

// === GENERIC TYPE EXPRESSION TESTS ===

#[test]
fn test_parse_generic_type_expr_method_call() {
    // Repository<User>.find(1) - generic type followed by method call
    let expr = parse_expr_test("Repository<User>.find(1)");
    match &expr.kind {
        ExprKind::MethodCall { receiver, method, type_args: _, args } => {
            assert_eq!(method.name.as_str(), "find", "Expected method name 'find'");
            assert_eq!(args.len(), 1, "Expected 1 argument");
            // Receiver should be a TypeExpr with a Generic type
            match &receiver.kind {
                ExprKind::TypeExpr(ty) => {
                    match &ty.kind {
                        verum_ast::TypeKind::Generic { base, args } => {
                            assert_eq!(args.len(), 1, "Expected 1 type argument");
                        }
                        _ => panic!("Expected Generic type, got {:?}", ty.kind),
                    }
                }
                _ => panic!("Expected TypeExpr receiver, got {:?}", receiver.kind),
            }
        }
        _ => panic!("Expected MethodCall expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_parse_generic_type_expr_nested() {
    // Map<Text, List<Int>>.new() - nested generic types
    let expr = parse_expr_test("Map<Text, List<Int>>.new()");
    match &expr.kind {
        ExprKind::MethodCall { receiver, method, type_args: _, args } => {
            assert_eq!(method.name.as_str(), "new", "Expected method name 'new'");
            assert_eq!(args.len(), 0, "Expected 0 arguments");
            // Receiver should be a TypeExpr with a Generic type
            match &receiver.kind {
                ExprKind::TypeExpr(ty) => {
                    match &ty.kind {
                        verum_ast::TypeKind::Generic { base, args } => {
                            assert_eq!(args.len(), 2, "Expected 2 type arguments for Map");
                        }
                        _ => panic!("Expected Generic type, got {:?}", ty.kind),
                    }
                }
                _ => panic!("Expected TypeExpr receiver, got {:?}", receiver.kind),
            }
        }
        _ => panic!("Expected MethodCall expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_parse_comparison_not_confused_with_generic() {
    // a < b should still be parsed as comparison
    let expr = parse_expr_test("a < b");
    match &expr.kind {
        ExprKind::Binary { op, left, right } => {
            assert_eq!(*op, BinOp::Lt, "Expected less-than operator");
        }
        _ => panic!("Expected Binary expression, got {:?}", expr.kind),
    }
}
