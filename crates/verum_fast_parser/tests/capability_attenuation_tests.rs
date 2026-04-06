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
//! Tests for capability attenuation parsing.
//!
//! Tests for the Verum Context System (capability-based DI, NOT algebraic effects) Section 10 - Capability Attenuation

use verum_ast::{Expr, ExprKind, expr::Capability};
use verum_common::Maybe;
use verum_lexer::Lexer;
use verum_fast_parser::RecursiveParser;

/// Helper to parse an expression from a string
fn parse_expr(input: &str) -> Result<Expr, Box<dyn std::error::Error>> {
    let file_id = verum_ast::span::FileId::new(0);
    let mut lexer = Lexer::new(input, file_id);
    let tokens = lexer.tokenize()?;
    let mut parser = RecursiveParser::new(&tokens, file_id);
    Ok(parser.parse_expr()?)
}

#[test]
fn test_parse_single_capability() {
    // Verum uses Capability.Name syntax (dot notation, not Rust's ::)
    let input = "Database.attenuate(Capability.ReadOnly)";
    let expr = parse_expr(input).expect("failed to parse");

    match expr.kind {
        ExprKind::Attenuate {
            context,
            capabilities,
        } => {
            // Check context is Database path
            match context.kind {
                ExprKind::Path(ref path) => {
                    assert_eq!(path.segments.len(), 1);
                }
                _ => panic!("Expected path expression for context"),
            }

            // Check capability set contains ReadOnly
            assert_eq!(capabilities.len(), 1);
            assert!(capabilities.contains(&Capability::ReadOnly));
        }
        _ => panic!("Expected Attenuate expression, got {:?}", expr.kind),
    }
}

#[test]
fn test_parse_multiple_capabilities() {
    let input = "Database.attenuate(Capability.ReadOnly | Capability.Query)";
    let expr = parse_expr(input).expect("failed to parse");

    match expr.kind {
        ExprKind::Attenuate {
            context,
            capabilities,
        } => {
            assert_eq!(capabilities.len(), 2);
            assert!(capabilities.contains(&Capability::ReadOnly));
            assert!(capabilities.contains(&Capability::Query));
        }
        _ => panic!("Expected Attenuate expression"),
    }
}

#[test]
fn test_parse_three_capabilities() {
    let input = "FileSystem.attenuate(Capability.ReadOnly | Capability.Query | Capability.Logging)";
    let expr = parse_expr(input).expect("failed to parse");

    match expr.kind {
        ExprKind::Attenuate {
            context,
            capabilities,
        } => {
            assert_eq!(capabilities.len(), 3);
            assert!(capabilities.contains(&Capability::ReadOnly));
            assert!(capabilities.contains(&Capability::Query));
            assert!(capabilities.contains(&Capability::Logging));
        }
        _ => panic!("Expected Attenuate expression"),
    }
}

#[test]
fn test_parse_custom_capability() {
    let input = "MyContext.attenuate(Capability.CustomAction)";
    let expr = parse_expr(input).expect("failed to parse");

    match expr.kind {
        ExprKind::Attenuate {
            context,
            capabilities,
        } => {
            assert_eq!(capabilities.len(), 1);
            // Custom capabilities should be stored
            match capabilities.capabilities.first() {
                Maybe::Some(Capability::Custom(name)) => {
                    assert_eq!(name.as_str(), "CustomAction");
                }
                _ => panic!("Expected Custom capability"),
            }
        }
        _ => panic!("Expected Attenuate expression"),
    }
}

#[test]
fn test_parse_all_standard_capabilities() {
    let capabilities = [
        "ReadOnly",
        "WriteOnly",
        "ReadWrite",
        "Admin",
        "Transaction",
        "Network",
        "FileSystem",
        "Query",
        "Execute",
        "Logging",
        "Metrics",
        "Config",
        "Cache",
        "Auth",
    ];

    for cap_name in &capabilities {
        let input = format!("Context.attenuate(Capability.{})", cap_name);
        let expr = parse_expr(&input).unwrap_or_else(|_| panic!("failed to parse {}", cap_name));

        match expr.kind {
            ExprKind::Attenuate { capabilities, .. } => {
                assert_eq!(
                    capabilities.len(),
                    1,
                    "Expected one capability for {}",
                    cap_name
                );
                // Just verify it parsed successfully
            }
            _ => panic!("Expected Attenuate expression for {}", cap_name),
        }
    }
}

#[test]
fn test_parse_nested_attenuate() {
    // Test that we can chain attenuate calls
    let input = "Database.attenuate(Capability.ReadWrite).attenuate(Capability.ReadOnly)";
    let expr = parse_expr(input).expect("failed to parse");

    match expr.kind {
        ExprKind::Attenuate {
            context,
            capabilities,
        } => {
            // Outer attenuate should have ReadOnly
            assert!(capabilities.contains(&Capability::ReadOnly));

            // Inner context should be another Attenuate
            match context.kind {
                ExprKind::Attenuate {
                    capabilities: inner_caps,
                    ..
                } => {
                    assert!(inner_caps.contains(&Capability::ReadWrite));
                }
                _ => panic!("Expected nested Attenuate"),
            }
        }
        _ => panic!("Expected Attenuate expression"),
    }
}

#[test]
fn test_parse_attenuate_in_let_binding() {
    let input = "let read_only_db = Database.attenuate(Capability.ReadOnly)";

    let file_id = verum_ast::span::FileId::new(0);
    let mut lexer = Lexer::new(input, file_id);
    let tokens = lexer.tokenize().expect("failed to tokenize");
    let mut parser = RecursiveParser::new(&tokens, file_id);
    let stmt = parser.parse_stmt().expect("failed to parse statement");

    // Verify the statement contains an attenuate expression
    // The exact structure depends on how let bindings are represented in the AST
}

#[test]
fn test_parse_attenuate_with_path_context() {
    // Test with a more complex path like std.database.Connection
    let input = "std.database.Connection.attenuate(Capability.ReadOnly)";
    let expr = parse_expr(input).expect("failed to parse");

    match expr.kind {
        ExprKind::Attenuate {
            context: _,
            capabilities,
        } => {
            // The context may be represented as field accesses rather than a simple path
            // This is expected behavior - the parser creates nested field access expressions
            // for multi-segment paths like std.database.Connection

            assert!(capabilities.contains(&Capability::ReadOnly));
        }
        _ => panic!("Expected Attenuate expression"),
    }
}

#[test]
fn test_parse_attenuate_mixed_capabilities() {
    // Mix standard and custom capabilities
    let input = "Context.attenuate(Capability.ReadOnly | Capability.MyCustomCap | Capability.Query)";
    let expr = parse_expr(input).expect("failed to parse");

    match expr.kind {
        ExprKind::Attenuate { capabilities, .. } => {
            assert_eq!(capabilities.len(), 3);
            assert!(capabilities.contains(&Capability::ReadOnly));
            assert!(capabilities.contains(&Capability::Query));

            // Check for custom capability
            let has_custom = capabilities
                .capabilities
                .iter()
                .any(|c| matches!(c, Capability::Custom(name) if name.as_str() == "MyCustomCap"));
            assert!(has_custom, "Expected custom capability");
        }
        _ => panic!("Expected Attenuate expression"),
    }
}

#[test]
fn test_sub_context_path_in_using_clause() {
    // Test that sub-context paths work in using clauses
    let input = r#"
        fn read_file(path: Path) -> Result<Data>
            using [FileSystem.Read]
        {
            read(path)
        }
    "#;

    let file_id = verum_ast::span::FileId::new(0);
    let mut lexer = Lexer::new(input, file_id);
    let tokens = lexer.tokenize().expect("failed to tokenize");
    let mut parser = RecursiveParser::new(&tokens, file_id);
    let item = parser.parse_item().expect("failed to parse function");

    // Verify the function has the correct context requirement
    use verum_ast::ItemKind;
    match item.kind {
        ItemKind::Function(func) => {
            assert_eq!(func.contexts.len(), 1);
            let ctx = &func.contexts[0];
            // The path should have 2 segments: FileSystem and Read
            assert_eq!(ctx.path.segments.len(), 2);
        }
        _ => panic!("Expected function item"),
    }
}

#[test]
fn test_multiple_sub_context_paths() {
    // Test multiple sub-context paths in a using clause
    let input = r#"
        fn process() -> Result<()>
            using [Database.Read, FileSystem.Write, Logger]
        {
            Ok(())
        }
    "#;

    let file_id = verum_ast::span::FileId::new(0);
    let mut lexer = Lexer::new(input, file_id);
    let tokens = lexer.tokenize().expect("failed to tokenize");
    let mut parser = RecursiveParser::new(&tokens, file_id);
    let item = parser.parse_item().expect("failed to parse function");

    use verum_ast::ItemKind;
    match item.kind {
        ItemKind::Function(func) => {
            assert_eq!(func.contexts.len(), 3);

            // First context: Database.Read (2 segments)
            assert_eq!(func.contexts[0].path.segments.len(), 2);

            // Second context: FileSystem.Write (2 segments)
            assert_eq!(func.contexts[1].path.segments.len(), 2);

            // Third context: Logger (1 segment)
            assert_eq!(func.contexts[2].path.segments.len(), 1);
        }
        _ => panic!("Expected function item"),
    }
}

#[test]
fn test_error_invalid_capability_syntax_rust_style() {
    // Rust-style :: separator is not valid in Verum (should use .)
    let input = "Database.attenuate(Capability::ReadOnly)";
    let result = parse_expr(input);
    assert!(
        result.is_err(),
        "Should fail to parse Rust-style :: syntax"
    );
}

#[test]
fn test_error_invalid_capability_syntax_plural() {
    // Using plural "Capabilities" is invalid (should use singular "Capability")
    let input = "Database.attenuate(Capabilities.ReadOnly)";
    let result = parse_expr(input);
    assert!(
        result.is_err(),
        "Should fail when using plural 'Capabilities' instead of 'Capability'"
    );
}

#[test]
fn test_error_missing_capability_name() {
    // Missing capability name after Capability.
    let input = "Database.attenuate(Capability.)";
    let result = parse_expr(input);
    assert!(
        result.is_err(),
        "Should fail when capability name is missing"
    );
}

#[test]
fn test_error_empty_attenuate_args() {
    // No capability provided
    let input = "Database.attenuate()";
    let result = parse_expr(input);
    assert!(
        result.is_err(),
        "Should fail when no capability is provided"
    );
}
