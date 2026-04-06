//! Tests for staged metaprogramming syntax: `meta(N) fn`.
//!
//! # Staged Metaprogramming Overview
//!
//! Verum supports N-level staged metaprogramming where functions execute
//! at different compilation stages:
//!
//! - **Stage 0**: Runtime execution (normal functions)
//! - **Stage 1**: Compile-time execution (`meta fn`, most common)
//! - **Stage N**: N-th level meta (`meta(N) fn`, generates Stage N-1 code)
//!
//! # Stage Coherence Rule
//!
//! A Stage N function can only DIRECTLY generate Stage N-1 code.
//! To generate lower-stage code, the output must contain meta functions
//! that perform further generation.

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

use verum_ast::{FileId, ItemKind, Module};
use verum_common::List;
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

/// Helper to parse a module from source.
fn parse_module(source: &str) -> Result<Module, String> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser.parse_module(lexer, file_id).map_err(|errors| {
        errors
            .into_iter()
            .map(|e| format!("{:?}", e))
            .collect::<Vec<_>>()
            .join(", ")
    })
}

/// Test basic `meta fn` parsing (stage 1 by default).
#[test]
fn test_meta_fn_default_stage() {
    let source = r#"
        meta fn derive_eq<T>() -> TokenStream {
            quote { }
        }
    "#;

    let module = parse_module(source).expect("Failed to parse");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(func.is_meta, "Function should be meta");
            assert_eq!(func.stage_level, 1, "Default meta stage should be 1");
            assert_eq!(func.name.name.as_str(), "derive_eq");
        }
        _ => panic!("Expected function declaration"),
    }
}

/// Test explicit `meta(1) fn` parsing (same as `meta fn`).
#[test]
fn test_meta_explicit_stage_1() {
    let source = r#"
        meta(1) fn derive_debug<T>() -> TokenStream {
            quote { }
        }
    "#;

    let module = parse_module(source).expect("Failed to parse");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(func.is_meta, "Function should be meta");
            assert_eq!(func.stage_level, 1, "Explicit meta(1) stage should be 1");
            assert_eq!(func.name.name.as_str(), "derive_debug");
        }
        _ => panic!("Expected function declaration"),
    }
}

/// Test `meta(2) fn` parsing (stage 2 - generates meta functions).
#[test]
fn test_meta_stage_2() {
    let source = r#"
        meta(2) fn create_derive_family() -> TokenStream {
            quote { }
        }
    "#;

    let module = parse_module(source).expect("Failed to parse");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(func.is_meta, "Function should be meta");
            assert_eq!(func.stage_level, 2, "meta(2) stage should be 2");
            assert_eq!(func.name.name.as_str(), "create_derive_family");
        }
        _ => panic!("Expected function declaration"),
    }
}

/// Test `meta(3) fn` parsing (stage 3 - meta-meta-programming).
#[test]
fn test_meta_stage_3() {
    let source = r#"
        meta(3) fn dsl_compiler_generator() -> TokenStream {
            quote { }
        }
    "#;

    let module = parse_module(source).expect("Failed to parse");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(func.is_meta, "Function should be meta");
            assert_eq!(func.stage_level, 3, "meta(3) stage should be 3");
            assert_eq!(func.name.name.as_str(), "dsl_compiler_generator");
        }
        _ => panic!("Expected function declaration"),
    }
}

/// Test normal function has stage 0.
#[test]
fn test_normal_fn_stage_0() {
    let source = r#"
        fn regular_function(x: Int) -> Int {
            x + 1
        }
    "#;

    let module = parse_module(source).expect("Failed to parse");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(!func.is_meta, "Function should not be meta");
            assert_eq!(func.stage_level, 0, "Normal function stage should be 0");
            assert_eq!(func.name.name.as_str(), "regular_function");
        }
        _ => panic!("Expected function declaration"),
    }
}

/// Test `pure meta fn` combination.
#[test]
fn test_pure_meta_fn() {
    let source = r#"
        pure meta fn const_fold() -> TokenStream {
            quote { }
        }
    "#;

    let module = parse_module(source).expect("Failed to parse");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(func.is_pure, "Function should be pure");
            assert!(func.is_meta, "Function should be meta");
            assert_eq!(func.stage_level, 1, "Default meta stage should be 1");
        }
        _ => panic!("Expected function declaration"),
    }
}

/// Test `pure meta(2) fn` combination.
#[test]
fn test_pure_meta_stage_2() {
    let source = r#"
        pure meta(2) fn advanced_codegen() -> TokenStream {
            quote { }
        }
    "#;

    let module = parse_module(source).expect("Failed to parse");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(func.is_pure, "Function should be pure");
            assert!(func.is_meta, "Function should be meta");
            assert_eq!(func.stage_level, 2, "Explicit stage should be 2");
        }
        _ => panic!("Expected function declaration"),
    }
}

/// Test `meta async fn` combination.
#[test]
fn test_meta_async_fn() {
    let source = r#"
        meta async fn async_codegen() -> TokenStream {
            quote { }
        }
    "#;

    let module = parse_module(source).expect("Failed to parse");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(func.is_meta, "Function should be meta");
            assert!(func.is_async, "Function should be async");
            assert_eq!(func.stage_level, 1, "Default meta stage should be 1");
        }
        _ => panic!("Expected function declaration"),
    }
}

/// Test multiple staged meta functions.
#[test]
fn test_multiple_staged_functions() {
    let source = r#"
        // Stage 0: Runtime
        fn runtime_fn() {}

        // Stage 1: Compile-time
        meta fn stage1_fn() {}

        // Stage 2: Meta-meta
        meta(2) fn stage2_fn() {}

        // Stage 3: Meta-meta-meta
        meta(3) fn stage3_fn() {}
    "#;

    let module = parse_module(source).expect("Failed to parse");
    assert_eq!(module.items.len(), 4);

    // Check stage 0
    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(!func.is_meta);
            assert_eq!(func.stage_level, 0);
        }
        _ => panic!("Expected function"),
    }

    // Check stage 1
    match &module.items[1].kind {
        ItemKind::Function(func) => {
            assert!(func.is_meta);
            assert_eq!(func.stage_level, 1);
        }
        _ => panic!("Expected function"),
    }

    // Check stage 2
    match &module.items[2].kind {
        ItemKind::Function(func) => {
            assert!(func.is_meta);
            assert_eq!(func.stage_level, 2);
        }
        _ => panic!("Expected function"),
    }

    // Check stage 3
    match &module.items[3].kind {
        ItemKind::Function(func) => {
            assert!(func.is_meta);
            assert_eq!(func.stage_level, 3);
        }
        _ => panic!("Expected function"),
    }
}

/// Test very high stage number.
#[test]
fn test_high_stage_number() {
    let source = r#"
        meta(10) fn extremely_meta() {}
    "#;

    let module = parse_module(source).expect("Failed to parse");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(func.is_meta);
            assert_eq!(func.stage_level, 10);
        }
        _ => panic!("Expected function"),
    }
}

/// Test meta function in protocol.
#[test]
fn test_meta_fn_in_protocol() {
    let source = r#"
        type Derive is protocol {
            meta fn to_tokens(&self) -> TokenStream;
        };
    "#;

    let module = parse_module(source).expect("Failed to parse");
    assert_eq!(module.items.len(), 1);

    // Just verify it parses without error
    // The actual structure access requires proper type matching
    match &module.items[0].kind {
        ItemKind::Type(_) => {
            // Protocol with meta function parsed successfully
        }
        _ => panic!("Expected type declaration"),
    }
}

/// Test meta function in implement block.
#[test]
fn test_meta_fn_in_impl() {
    let source = r#"
        implement Derive for MyType {
            meta fn to_tokens(&self) -> TokenStream {
                quote { }
            }
        }
    "#;

    let module = parse_module(source).expect("Failed to parse");
    assert_eq!(module.items.len(), 1);

    // Just verify it parses without error
    match &module.items[0].kind {
        ItemKind::Impl(_) => {
            // Impl with meta function parsed successfully
        }
        _ => panic!("Expected implement declaration"),
    }
}

/// Test higher stage meta function in implement block.
#[test]
fn test_meta_stage_2_in_impl() {
    let source = r#"
        implement AdvancedDerive for MyType {
            meta(2) fn generate_derives(&self) -> TokenStream {
                quote { }
            }
        }
    "#;

    let module = parse_module(source).expect("Failed to parse");
    assert_eq!(module.items.len(), 1);

    // Just verify it parses without error
    match &module.items[0].kind {
        ItemKind::Impl(_) => {
            // Impl with meta(2) function parsed successfully
        }
        _ => panic!("Expected implement declaration"),
    }
}

// ============================================================================
// Quote Expression Parsing Tests
// ============================================================================

use verum_ast::expr::ExprKind;

/// Test basic `quote { tokens }` expression parsing.
#[test]
fn test_quote_basic_expression() {
    let source = r#"
        meta fn generate_add() -> TokenStream {
            quote { fn add(a: Int, b: Int) -> Int { a + b } }
        }
    "#;

    let module = parse_module(source).expect("Failed to parse quote expression");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(func.is_meta, "Function should be meta");
            // Function parsed with quote body
            assert!(func.body.is_some(), "Body should be present");
        }
        _ => panic!("Expected function declaration"),
    }
}

/// Test `quote(N)` expression with explicit target stage.
#[test]
fn test_quote_with_target_stage() {
    let source = r#"
        meta(2) fn generate_meta_macro() -> TokenStream {
            quote(1) { meta fn inner() {} }
        }
    "#;

    let module = parse_module(source).expect("Failed to parse quote(N) expression");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(func.is_meta, "Function should be meta");
            assert_eq!(func.stage_level, 2, "Stage should be 2");
        }
        _ => panic!("Expected function declaration"),
    }
}

/// Test `quote(0)` for generating runtime code.
#[test]
fn test_quote_target_stage_0() {
    let source = r#"
        meta fn generate_runtime_code() -> TokenStream {
            quote(0) { fn runtime_func() {} }
        }
    "#;

    let module = parse_module(source).expect("Failed to parse quote(0)");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(func.is_meta, "Function should be meta");
        }
        _ => panic!("Expected function declaration"),
    }
}

/// Test quote with nested braces.
#[test]
fn test_quote_nested_braces() {
    let source = r#"
        meta fn generate_nested() -> TokenStream {
            quote {
                fn outer() {
                    fn inner() {
                        let x = { 1 + 2 };
                    }
                }
            }
        }
    "#;

    let module = parse_module(source).expect("Failed to parse quote with nested braces");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(func.is_meta, "Function should be meta");
        }
        _ => panic!("Expected function declaration"),
    }
}

/// Test quote with complex expressions.
#[test]
fn test_quote_complex_content() {
    let source = r#"
        meta fn generate_complex() -> TokenStream {
            quote {
                type Point is { x: Float, y: Float };
                implement Point {
                    fn new(x: Float, y: Float) -> Point {
                        Point { x: x, y: y }
                    }
                }
            }
        }
    "#;

    let module = parse_module(source).expect("Failed to parse complex quote");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(func.is_meta, "Function should be meta");
        }
        _ => panic!("Expected function declaration"),
    }
}

/// Test quote in let binding.
#[test]
fn test_quote_in_let() {
    let source = r#"
        meta fn generate_tokens() -> TokenStream {
            let tokens = quote { let x = 42; };
            tokens
        }
    "#;

    let module = parse_module(source).expect("Failed to parse quote in let");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(func.is_meta, "Function should be meta");
        }
        _ => panic!("Expected function declaration"),
    }
}

/// Test multiple quote expressions.
#[test]
fn test_multiple_quotes() {
    let source = r#"
        meta fn combine_quotes() -> TokenStream {
            let part1 = quote { fn foo() {} };
            let part2 = quote { fn bar() {} };
            concat(part1, part2)
        }
    "#;

    let module = parse_module(source).expect("Failed to parse multiple quotes");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(func.is_meta, "Function should be meta");
        }
        _ => panic!("Expected function declaration"),
    }
}

/// Test empty quote expression.
#[test]
fn test_quote_empty() {
    let source = r#"
        meta fn empty_quote() -> TokenStream {
            quote {}
        }
    "#;

    let module = parse_module(source).expect("Failed to parse empty quote");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(func.is_meta, "Function should be meta");
        }
        _ => panic!("Expected function declaration"),
    }
}

// ============================================================================
// Stage Escape Expression Parsing Tests
// ============================================================================

/// Test basic `$(stage 0){ expr }` parsing.
#[test]
fn test_stage_escape_basic() {
    let source = r#"
        meta fn generate_with_escape() -> TokenStream {
            let value = 42;
            quote { let x = $(stage 0){ value }; }
        }
    "#;

    let module = parse_module(source).expect("Failed to parse stage escape");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(func.is_meta, "Function should be meta");
            assert!(func.body.is_some(), "Body should be present");
        }
        _ => panic!("Expected function declaration"),
    }
}

/// Test `$(stage 1){ expr }` for higher stage escapes.
#[test]
fn test_stage_escape_stage_1() {
    let source = r#"
        meta(2) fn generate_meta() -> TokenStream {
            let name = "inner_fn";
            quote(1) {
                meta fn $(stage 1){ name }() -> TokenStream {
                    quote { }
                }
            }
        }
    "#;

    let module = parse_module(source).expect("Failed to parse stage 1 escape");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(func.is_meta, "Function should be meta");
            assert_eq!(func.stage_level, 2, "Stage should be 2");
        }
        _ => panic!("Expected function declaration"),
    }
}

/// Test stage escape with complex expression.
#[test]
fn test_stage_escape_complex_expr() {
    let source = r#"
        meta fn generate_computed() -> TokenStream {
            let base = 10;
            let offset = 5;
            quote { let result = $(stage 0){ base + offset }; }
        }
    "#;

    let module = parse_module(source).expect("Failed to parse complex stage escape");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(func.is_meta, "Function should be meta");
        }
        _ => panic!("Expected function declaration"),
    }
}

/// Test multiple stage escapes in one quote.
#[test]
fn test_multiple_stage_escapes() {
    let source = r#"
        meta fn generate_multiple() -> TokenStream {
            let x = 1;
            let y = 2;
            quote {
                let a = $(stage 0){ x };
                let b = $(stage 0){ y };
            }
        }
    "#;

    let module = parse_module(source).expect("Failed to parse multiple stage escapes");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(func.is_meta, "Function should be meta");
        }
        _ => panic!("Expected function declaration"),
    }
}

/// Test stage escape as standalone expression (outside quote for testing parser).
#[test]
fn test_stage_escape_standalone() {
    let source = r#"
        fn test_standalone() {
            let result = $(stage 0){ 42 };
        }
    "#;

    let module = parse_module(source).expect("Failed to parse standalone stage escape");
    assert_eq!(module.items.len(), 1);

    match &module.items[0].kind {
        ItemKind::Function(func) => {
            assert!(!func.is_meta, "Function should not be meta");
            assert_eq!(func.stage_level, 0, "Stage should be 0");
        }
        _ => panic!("Expected function declaration"),
    }
}
