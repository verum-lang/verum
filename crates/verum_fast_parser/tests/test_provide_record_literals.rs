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
// Tests for provide statement with various expression types including record literals
//
// This test verifies that provide statements can parse:
// 1. Simple values (identifiers)
// 2. Function calls
// 3. Record literals (struct initialization)
// 4. Complex nested expressions

use verum_ast::{ExprKind, FileId, FunctionBody, ItemKind, Spanned, Stmt, StmtKind};
use verum_common::{List, Maybe};
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

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

#[test]
fn test_provide_simple_value() {
    // Test 1: Simple value (identifier)
    let stmts = parse_stmt("provide Logger = simple_logger;");
    assert_eq!(stmts.len(), 1, "Expected one statement");

    if let StmtKind::Provide { context, value, .. } = &stmts[0].kind {
        assert_eq!(context.as_str(), "Logger");
        // Value should be a path expression
        assert!(matches!(value.kind, ExprKind::Path(_)));
    } else {
        panic!("Expected Provide statement, got: {:?}", stmts[0].kind);
    }
}

#[test]
fn test_provide_function_call() {
    // Test 2: Function call
    let stmts = parse_stmt("provide Logger = create_logger();");
    assert_eq!(stmts.len(), 1, "Expected one statement");

    if let StmtKind::Provide { context, value, .. } = &stmts[0].kind {
        assert_eq!(context.as_str(), "Logger");
        // Value should be a call expression
        assert!(matches!(value.kind, ExprKind::Call { .. }));
    } else {
        panic!("Expected Provide statement, got: {:?}", stmts[0].kind);
    }
}

#[test]
fn test_provide_record_literal() {
    // Test 3: Record literal (single field)
    let stmts = parse_stmt("provide Logger = ConsoleLogger { prefix: \"APP\" };");
    assert_eq!(stmts.len(), 1, "Expected one statement");

    if let StmtKind::Provide { context, value, .. } = &stmts[0].kind {
        assert_eq!(context.as_str(), "Logger");
        // Value should be a record expression
        if let ExprKind::Record { path, fields, base } = &value.kind {
            // Check that it's ConsoleLogger
            assert_eq!(path.segments.len(), 1);
            // Check that it has one field
            assert_eq!(fields.len(), 1);
            // Check that there's no base
            assert!(matches!(base, Maybe::None));
        } else {
            panic!("Expected Record expression, got: {:?}", value.kind);
        }
    } else {
        panic!("Expected Provide statement, got: {:?}", stmts[0].kind);
    }
}

#[test]
fn test_provide_record_multiple_fields() {
    // Test 4: Record literal with multiple fields
    let stmts = parse_stmt(
        r#"provide Database = DbConfig {
        host: "localhost",
        port: 5432,
        name: "mydb"
    };"#,
    );
    assert_eq!(stmts.len(), 1, "Expected one statement");

    if let StmtKind::Provide { context, value, .. } = &stmts[0].kind {
        assert_eq!(context.as_str(), "Database");
        // Value should be a record expression
        if let ExprKind::Record { path, fields, .. } = &value.kind {
            assert_eq!(path.segments.len(), 1);
            assert_eq!(fields.len(), 3, "Expected 3 fields: host, port, name");
        } else {
            panic!("Expected Record expression, got: {:?}", value.kind);
        }
    } else {
        panic!("Expected Provide statement, got: {:?}", stmts[0].kind);
    }
}

#[test]
fn test_provide_nested_record() {
    // Test 5: Nested record literal
    let stmts = parse_stmt(
        r#"provide Config = AppConfig {
        logger: ConsoleLogger { prefix: "APP" },
        timeout: 30
    };"#,
    );
    assert_eq!(stmts.len(), 1, "Expected one statement");

    if let StmtKind::Provide { context, value, .. } = &stmts[0].kind {
        assert_eq!(context.as_str(), "Config");
        // Value should be a record expression
        if let ExprKind::Record { path, fields, .. } = &value.kind {
            assert_eq!(path.segments.len(), 1);
            assert_eq!(fields.len(), 2, "Expected 2 fields: logger, timeout");

            // Check that the first field (logger) has a nested record as its value
            if let Maybe::Some(field_value) = &fields[0].value {
                assert!(
                    matches!(field_value.kind, ExprKind::Record { .. }),
                    "Expected nested record for logger field"
                );
            }
        } else {
            panic!("Expected Record expression, got: {:?}", value.kind);
        }
    } else {
        panic!("Expected Provide statement, got: {:?}", stmts[0].kind);
    }
}

#[test]
fn test_provide_with_shorthand_field() {
    // Test 6: Record with shorthand field syntax
    let stmts = parse_stmt("provide Logger = ConsoleLogger { prefix };");
    assert_eq!(stmts.len(), 1, "Expected one statement");

    if let StmtKind::Provide { context, value, .. } = &stmts[0].kind {
        assert_eq!(context.as_str(), "Logger");
        // Value should be a record expression
        if let ExprKind::Record { fields, .. } = &value.kind {
            assert_eq!(fields.len(), 1);
            // Shorthand field should have None as its value
            assert!(matches!(fields[0].value, Maybe::None));
        } else {
            panic!("Expected Record expression, got: {:?}", value.kind);
        }
    } else {
        panic!("Expected Provide statement, got: {:?}", stmts[0].kind);
    }
}

#[test]
fn test_provide_with_spread() {
    // Test 7: Record with spread/update syntax
    let stmts = parse_stmt("provide Logger = ConsoleLogger { prefix: \"NEW\", ..old };");
    assert_eq!(stmts.len(), 1, "Expected one statement");

    if let StmtKind::Provide { context, value, .. } = &stmts[0].kind {
        assert_eq!(context.as_str(), "Logger");
        // Value should be a record expression
        if let ExprKind::Record { fields, base, .. } = &value.kind {
            assert_eq!(fields.len(), 1);
            // Should have a base expression
            assert!(matches!(base, Maybe::Some(_)));
        } else {
            panic!("Expected Record expression, got: {:?}", value.kind);
        }
    } else {
        panic!("Expected Provide statement, got: {:?}", stmts[0].kind);
    }
}

#[test]
fn test_provide_qualified_path_record() {
    // Test 8: Record with qualified path (module::Type)
    let stmts = parse_stmt("provide Logger = std.io.ConsoleLogger { prefix: \"APP\" };");
    assert_eq!(stmts.len(), 1, "Expected one statement");

    if let StmtKind::Provide { context, value, .. } = &stmts[0].kind {
        assert_eq!(context.as_str(), "Logger");
        // Value should be a record expression
        if let ExprKind::Record { path, fields, .. } = &value.kind {
            // Should have multiple segments for qualified path
            assert!(path.segments.len() > 1, "Expected qualified path");
            assert_eq!(fields.len(), 1);
        } else {
            panic!("Expected Record expression, got: {:?}", value.kind);
        }
    } else {
        panic!("Expected Provide statement, got: {:?}", stmts[0].kind);
    }
}
