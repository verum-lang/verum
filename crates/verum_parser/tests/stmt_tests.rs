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
// Tests for statement parsing
//
// Tests for statement parsing: let, provide, defer, for, while, loop, return, etc.
// This module tests parsing of all Verum statement forms including:
// - Let bindings with and without type annotations
// - Let-else statements
// - Expression statements
// - Defer statements
// - Provide statements

use verum_ast::{FileId, FunctionBody, ItemKind, Spanned, Stmt, StmtKind};
use verum_common::{List, Maybe};
use verum_lexer::Lexer;
use verum_parser::VerumParser;

/// Parse a statement by wrapping it in a function body
fn parse_stmt(source: &str) -> List<Stmt> {
    // Wrap statement in a function body
    let wrapped = format!("fn __test__() {{ {} }}", source);
    let file_id = FileId::new(0);
    let lexer = Lexer::new(&wrapped, file_id);
    let parser = VerumParser::new();
    let module = parser
        .parse_module(lexer, file_id)
        .unwrap_or_else(|_| panic!("Failed to parse: {}", source));

    // Extract the function body statements
    if let Some(item) = module.items.get(0)
        && let ItemKind::Function(func) = &item.kind
            && let Some(FunctionBody::Block(block)) = &func.body {
                let mut stmts = block.stmts.clone();

                // If there's a trailing expression (no semicolon), add it as an expression statement
                if let Maybe::Some(expr) = &block.expr {
                    let span = expr.span();
                    let expr_stmt = Stmt {
                        kind: StmtKind::Expr {
                            expr: (**expr).clone(),
                            has_semi: false,
                        },
                        span,
                        attributes: Vec::new(),
                    };
                    stmts.push(expr_stmt);
                }

                return stmts;
            }

    panic!("Failed to extract statements from function body");
}

// === LET BINDING TESTS ===

#[test]
fn test_parse_let_binding_simple() {
    let stmts = parse_stmt("let x = 5;");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_let_binding_with_type() {
    let stmts = parse_stmt("let x: Int = 5;");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_let_binding_without_value() {
    let stmts = parse_stmt("let x: Int;");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_let_binding_pattern_tuple() {
    let stmts = parse_stmt("let (x, y) = (1, 2);");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_let_binding_mutable() {
    let stmts = parse_stmt("let mut x = 5;");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_multiple_let_bindings() {
    let stmts = parse_stmt("let x = 1; let y = 2; let z = 3;");
    assert_eq!(stmts.len(), 3, "Expected three statements");
}

#[test]
fn test_parse_let_binding_complex_expression() {
    let stmts = parse_stmt("let x = 1 + 2 * 3;");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

// === LET-ELSE TESTS ===

#[test]
fn test_parse_let_else_simple() {
    let stmts = parse_stmt("let Some(x) = value else { return };");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_let_else_with_type() {
    let stmts = parse_stmt("let Some(x): Option<Int> = value else { return };");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_let_else_block() {
    let stmts = parse_stmt("let x = y else { continue };");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

// === DEFER TESTS ===
// NOTE: Defer statements are not yet implemented in the parser

#[test]
fn test_parse_defer_simple() {
    let stmts = parse_stmt("defer cleanup();");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_defer_block() {
    let stmts = parse_stmt("defer { close(file); flush(); };");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_multiple_defers() {
    let stmts = parse_stmt("defer a(); defer b(); defer c();");
    assert_eq!(stmts.len(), 3, "Expected three statements");
}

// === PROVIDE TESTS ===
// Tests for context provider statements

#[test]
fn test_parse_provide_simple() {
    let stmts = parse_stmt("provide Logger = default_logger();");
    assert_eq!(stmts.len(), 1, "Expected one statement");

    match &stmts.get(0).unwrap().kind {
        StmtKind::Provide { context, .. } => {
            assert_eq!(context.as_str(), "Logger");
        }
        _ => panic!("Expected Provide statement"),
    }
}

#[test]
fn test_parse_provide_with_identifier() {
    let stmts = parse_stmt("provide Database = db_connection;");
    assert_eq!(stmts.len(), 1, "Expected one statement");

    match &stmts.get(0).unwrap().kind {
        StmtKind::Provide { context, .. } => {
            assert_eq!(context.as_str(), "Database");
        }
        _ => panic!("Expected Provide statement"),
    }
}

#[test]
fn test_parse_provide_with_constructor() {
    let stmts = parse_stmt("provide Logger = ConsoleLogger { level: Level.Debug };");
    assert_eq!(stmts.len(), 1, "Expected one statement");

    match &stmts.get(0).unwrap().kind {
        StmtKind::Provide { context, .. } => {
            assert_eq!(context.as_str(), "Logger");
        }
        _ => panic!("Expected Provide statement"),
    }
}

#[test]
fn test_parse_provide_with_async_call() {
    let stmts = parse_stmt("provide Database = connect_to_db().await;");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_provide_path_based_single() {
    let stmts = parse_stmt("provide FileSystem.Write = writer_impl;");
    assert_eq!(stmts.len(), 1, "Expected one statement");

    match &stmts.get(0).unwrap().kind {
        StmtKind::Provide { context, .. } => {
            assert_eq!(context.as_str(), "FileSystem.Write");
        }
        _ => panic!("Expected Provide statement"),
    }
}

#[test]
fn test_parse_provide_path_based_nested() {
    let stmts = parse_stmt("provide Database.Connection.Pool = pool_impl;");
    assert_eq!(stmts.len(), 1, "Expected one statement");

    match &stmts.get(0).unwrap().kind {
        StmtKind::Provide { context, .. } => {
            assert_eq!(context.as_str(), "Database.Connection.Pool");
        }
        _ => panic!("Expected Provide statement"),
    }
}

#[test]
fn test_parse_provide_with_method_chain() {
    let stmts = parse_stmt("provide Logger = ConsoleLogger.new().with_level(Level.Info);");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_provide_semicolon_optional_before_keyword() {
    // Semicolon can be omitted when followed by statement-starting keyword
    let stmts = parse_stmt("provide Logger = logger let x = 1;");
    assert_eq!(stmts.len(), 2, "Expected two statements");
    assert!(matches!(
        stmts.get(0).unwrap().kind,
        StmtKind::Provide { .. }
    ));
    assert!(matches!(stmts.get(1).unwrap().kind, StmtKind::Let { .. }));
}

#[test]
fn test_parse_multiple_provides() {
    let stmts = parse_stmt("provide Database = db; provide Logger = log; provide Cache = cache;");
    assert_eq!(stmts.len(), 3, "Expected three statements");

    for stmt in stmts.iter() {
        assert!(matches!(stmt.kind, StmtKind::Provide { .. }));
    }
}

#[test]
fn test_parse_provide_with_complex_expression() {
    let stmts = parse_stmt(
        "provide Logger = if debug { DebugLogger.new() } else { ProductionLogger.new() };",
    );
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_provide_with_match() {
    let stmts = parse_stmt(
        "provide Logger = match env { Env.Dev => dev_logger(), Env.Prod => prod_logger() };",
    );
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_provide_with_block_value() {
    let stmts = parse_stmt("provide Logger = { let l = ConsoleLogger.new(); l.configure(); l };");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_provide_for_generic_context() {
    // Context names don't include type parameters - generics are inferred
    // Correct: provide Cache = RedisCache.new();
    // Not: provide Cache<Text, User> = RedisCache.new();
    let stmts = parse_stmt("provide Cache = RedisCache.new();");
    assert_eq!(stmts.len(), 1, "Expected one statement");

    match &stmts.get(0).unwrap().kind {
        StmtKind::Provide { context, .. } => {
            assert_eq!(context.as_str(), "Cache");
        }
        _ => panic!("Expected Provide statement"),
    }
}

#[test]
fn test_parse_provide_in_block_scope() {
    // Simulates scoped provide pattern from spec
    let stmts = parse_stmt("{ provide Logger = file_logger(); Logger.log(Level.Info, msg); }");
    assert_eq!(stmts.len(), 1, "Expected one statement (block)");
}

// === PROVIDE ERROR HANDLING TESTS ===

#[test]
#[should_panic(expected = "Failed to parse")]
fn test_parse_provide_missing_equals() {
    // Missing = should fail
    parse_stmt("provide Logger logger;");
}

#[test]
#[should_panic(expected = "Failed to parse")]
fn test_parse_provide_missing_value() {
    // Missing value expression should fail
    parse_stmt("provide Logger = ;");
}

#[test]
#[should_panic(expected = "Failed to parse")]
fn test_parse_provide_missing_context_name() {
    // Missing context name should fail
    parse_stmt("provide = logger;");
}

#[test]
#[should_panic(expected = "Failed to parse")]
fn test_parse_provide_invalid_context_path() {
    // Invalid path (ends with dot) should fail
    parse_stmt("provide FileSystem. = impl;");
}

// === EXPRESSION STATEMENTS ===

#[test]
fn test_parse_expression_statement_with_semicolon() {
    let stmts = parse_stmt("1 + 2;");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_expression_statement_without_semicolon() {
    let stmts = parse_stmt("1 + 2");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_function_call_statement() {
    let stmts = parse_stmt("foo();");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_method_call_statement() {
    let stmts = parse_stmt("obj.method();");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_assignment_statement() {
    let stmts = parse_stmt("x = 5;");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

// === COMPOUND STATEMENTS ===

#[test]
fn test_parse_statements_sequence() {
    let stmts = parse_stmt("let x = 1; let y = 2; let z = 3;");
    assert_eq!(stmts.len(), 3, "Expected three statements");
}

#[test]
fn test_parse_mixed_statement_types() {
    // NOTE: Includes defer which is not yet implemented
    let stmts = parse_stmt("let x = 1; defer cleanup(); x + 1;");
    assert_eq!(stmts.len(), 3, "Expected three statements");
}

// === BLOCK STATEMENTS ===

#[test]
fn test_parse_block_with_statements() {
    let stmts = parse_stmt("{ let x = 1; let y = 2; }");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

// === CONTROL FLOW IN STATEMENTS ===

#[test]
fn test_parse_if_statement() {
    let stmts = parse_stmt("if x > 0 { y = 1 } else { y = 2 };");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_match_statement() {
    let stmts = parse_stmt("match x { 1 => 2, _ => 3 };");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_loop_statement() {
    let stmts = parse_stmt("loop { break };");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_while_statement() {
    let stmts = parse_stmt("while x < 10 { x = x + 1 };");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_for_statement() {
    let stmts = parse_stmt("for x in items { y = x };");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

// === COMPLEX PATTERNS ===

#[test]
fn test_parse_let_with_wildcard_pattern() {
    let stmts = parse_stmt("let _ = value;");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_let_with_rest_pattern() {
    let stmts = parse_stmt("let [x, .., y] = arr;");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_let_with_or_pattern() {
    let stmts = parse_stmt("let x | y = value;");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

// === TYPE ANNOTATIONS ===

#[test]
fn test_parse_let_with_primitive_type() {
    let stmts = parse_stmt("let x: Int = 5;");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_let_with_generic_type() {
    let stmts = parse_stmt("let x: List<Int> = [];");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_let_with_refinement_type() {
    let stmts = parse_stmt("let x: Int{> 0} = 5;");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_let_with_reference_type() {
    let stmts = parse_stmt("let x: &Int = &y;");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_let_with_function_type() {
    let stmts = parse_stmt("let f: fn(Int) -> Int = |x| x;");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_let_with_tuple_type() {
    let stmts = parse_stmt("let p: (Int, Text) = (1, \"hello\");");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

// === EDGE CASES ===

#[test]
fn test_parse_empty_defer_block() {
    // NOTE: Defer is not yet implemented
    let stmts = parse_stmt("defer { };");
    assert_eq!(stmts.len(), 1, "Expected one statement");
}

#[test]
fn test_parse_assignment_operators() {
    let code = "x += 1; y -= 2; z *= 3; w /= 4; v %= 5;";
    let stmts = parse_stmt(code);
    assert_eq!(stmts.len(), 5, "Expected five statements");
}

#[test]
fn test_parse_bitwise_assignment() {
    let code = "x &= 1; y |= 2; z ^= 3; a <<= 1; b >>= 1;";
    let stmts = parse_stmt(code);
    assert_eq!(stmts.len(), 5, "Expected five statements");
}
