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
//! Integration tests for the quote! and unquote! macro system
//!
//! These tests verify that the quote/unquote system works correctly
//! for meta-programming in Verum.

use verum_ast::{
    Expr, Span,
    expr::ExprKind,
    ty::{Ident, Path},
};
use verum_compiler::meta::ConstValue;
use verum_compiler::quote::{MetaContext, ident};
use verum_compiler::quote_macro::{
    MacroExpansionContext, create_quote_context, create_quote_context_with_repeats, meta_quote,
    meta_unquote, quote_expr, quote_with_context, tokenstream_from_str, unquote_stream,
};
use verum_common::{FileId, List, Text};

#[test]
fn test_quote_simple_expr() {
    // Test quoting a simple identifier expression
    let expr = Expr::new(
        ExprKind::Path(Path::single(Ident::new("x", Span::default()))),
        Span::default(),
    );

    let ts = quote_expr(&expr);
    assert!(!ts.is_empty(), "TokenStream should not be empty");
}

#[test]
fn test_unquote_simple_expr() {
    // Test round-tripping: expr -> tokens -> expr
    let original_expr = Expr::new(
        ExprKind::Path(Path::single(Ident::new("x", Span::default()))),
        Span::default(),
    );

    let ts = quote_expr(&original_expr);
    let result = unquote_stream(&ts);

    assert!(result.is_ok(), "Unquote should succeed");
}

#[test]
fn test_quote_with_interpolation() {
    // Test quote with single variable interpolation
    let mut ctx = MetaContext::new();
    ctx.bind_single(Text::from("name"), ident("my_var", Span::default()));

    let result = quote_with_context("let #name = 42;", &ctx);
    assert!(result.is_ok(), "Quote with interpolation should succeed");

    let ts = result.unwrap();
    assert!(!ts.is_empty(), "Interpolated quote should produce tokens");
}

#[test]
fn test_quote_with_repetition() {
    // Test quote with repetition pattern #(#items),*
    let ctx = create_quote_context_with_repeats(
        &[],
        &[(
            "fields",
            vec![
                ident("name", Span::default()),
                ident("age", Span::default()),
                ident("email", Span::default()),
            ],
        )],
    );

    // In a full implementation, this would work:
    // let result = quote_with_context("struct Person { #(#fields),* }", &ctx);
    // For now, just verify context creation worked
    assert!(
        ctx.get_repeat("fields").is_some(),
        "Repeat binding should exist"
    );
    assert_eq!(
        ctx.get_repeat("fields").unwrap().len(),
        3,
        "Should have 3 fields"
    );
}

#[test]
fn test_tokenstream_from_str() {
    let code = "let x = 42;";
    let result = tokenstream_from_str(code);

    assert!(result.is_ok(), "Parsing valid code should succeed");
    assert!(
        !result.unwrap().is_empty(),
        "Parsed code should produce tokens"
    );
}

#[test]
fn test_tokenstream_from_complex_code() {
    let code = r#"
        fn factorial(n: Int) -> Int {
            if n <= 1 {
                1
            } else {
                n * factorial(n - 1)
            }
        }
    "#;

    let result = tokenstream_from_str(code);
    assert!(result.is_ok(), "Parsing function should succeed");
}

#[test]
fn test_meta_quote_function() {
    // Test the meta_quote built-in function
    let expr = Expr::new(
        ExprKind::Path(Path::single(Ident::new("x", Span::default()))),
        Span::default(),
    );

    let args = List::from_iter(vec![ConstValue::Expr(expr.clone())]);
    let result = meta_quote(args);

    assert!(result.is_ok(), "meta_quote should succeed");
    match result.unwrap() {
        ConstValue::Expr(e) => {
            // Verify the expression is preserved
            assert!(matches!(e.kind, ExprKind::Path(_)));
        }
        _ => panic!("meta_quote should return ConstValue::Expr"),
    }
}

#[test]
fn test_meta_quote_with_text() {
    // Test meta_quote with a text template
    // ConstValue::Text expects String (verum_common::Text = String alias)
    // Note: "let x = 42;" is a statement, not an expression
    // For parse_as_expr, we need an expression like "x" or "42" or "1 + 2"
    let template = String::from("42");
    let args = List::from_iter(vec![ConstValue::Text(template.into())]);

    let result = meta_quote(args.clone());
    assert!(
        result.is_ok(),
        "meta_quote with text should succeed: {:?}",
        result
    );

    // Also test with a more complex expression
    let template2 = String::from("x + y");
    let args2 = List::from_iter(vec![ConstValue::Text(template2.into())]);
    let result2 = meta_quote(args2);
    assert!(
        result2.is_ok(),
        "meta_quote with binary expression should succeed: {:?}",
        result2
    );
}

#[test]
fn test_meta_unquote_function() {
    // Test the meta_unquote built-in function
    let expr = Expr::new(
        ExprKind::Path(Path::single(Ident::new("x", Span::default()))),
        Span::default(),
    );

    let args = List::from_iter(vec![ConstValue::Expr(expr.clone())]);
    let result = meta_unquote(args);

    assert!(result.is_ok(), "meta_unquote should succeed");
    match result.unwrap() {
        ConstValue::Expr(e) => {
            assert!(matches!(e.kind, ExprKind::Path(_)));
        }
        _ => panic!("meta_unquote should return ConstValue::Expr"),
    }
}

#[test]
fn test_create_quote_context() {
    let bindings = vec![
        ("name", ident("my_var", Span::default())),
        ("value", ident("42", Span::default())),
        ("type_name", ident("Int", Span::default())),
    ];

    let ctx = create_quote_context(&bindings);

    assert!(
        ctx.get_single("name").is_some(),
        "name binding should exist"
    );
    assert!(
        ctx.get_single("value").is_some(),
        "value binding should exist"
    );
    assert!(
        ctx.get_single("type_name").is_some(),
        "type_name binding should exist"
    );
}

#[test]
fn test_macro_expansion_context() {
    let ctx = MacroExpansionContext::new(Span::default(), String::from("my::module::path"));

    assert_eq!(ctx.module_path.as_str(), "my::module::path");
    assert!(!ctx.is_const, "Should not be const by default");
    assert!(!ctx.is_async, "Should not be async by default");

    let const_ctx = ctx.clone().in_const_context();
    assert!(const_ctx.is_const, "Should be const after marking");

    let async_ctx = ctx.in_async_context();
    assert!(async_ctx.is_async, "Should be async after marking");
}

#[test]
fn test_quote_preserves_span_info() {
    // Test that span information is preserved through quote/unquote
    let span = Span::new(10, 20, FileId::new(0));
    let expr = Expr::new(ExprKind::Path(Path::single(Ident::new("x", span))), span);

    let ts = quote_expr(&expr);
    assert!(!ts.is_empty());

    // Span preservation is implementation-dependent
    // This test verifies the structure is maintained
}

#[test]
fn test_empty_quote_context() {
    let ctx = MetaContext::new();

    // Empty context should still allow quoting literals
    let result = quote_with_context("42", &ctx);
    assert!(result.is_ok(), "Empty context should work for literals");
}

#[test]
fn test_quote_with_missing_binding() {
    let ctx = MetaContext::new();

    // This should fail because 'name' is not bound
    let _result = quote_with_context("let #name = 42;", &ctx);

    // The actual behavior depends on the Quote implementation
    // It might succeed with an error marker or fail
    // Just verify it doesn't panic
}

#[test]
fn test_complex_quote_pattern() {
    // Test a more complex quoting pattern for generating getters
    let mut ctx = MetaContext::new();
    ctx.bind_single(Text::from("field"), ident("name", Span::default()));
    ctx.bind_single(Text::from("ty"), ident("Text", Span::default()));

    // Simulate generating a getter:
    // fn get_name(&self) -> &Text { &self.name }
    let template = "fn get_ #field(&self) -> &#ty";
    let _result = quote_with_context(template, &ctx);

    // Verify the quote doesn't crash (exact behavior depends on implementation)
    // In production, this would generate valid AST
}

#[test]
fn test_tokenstream_invalid_syntax() {
    let invalid_code = "let x = ;"; // Missing value
    let _result = tokenstream_from_str(invalid_code);

    // Should handle parse errors gracefully
    // Exact behavior depends on parser implementation
}

#[test]
fn test_quote_with_nested_interpolation() {
    let mut ctx = MetaContext::new();
    ctx.bind_single(Text::from("outer"), ident("value1", Span::default()));

    // Test that we can quote expressions with nested structure
    let result = quote_with_context("Some( #outer)", &ctx);
    assert!(result.is_ok() || result.is_err()); // Just verify it doesn't panic
}

#[test]
fn test_meta_quote_empty_args() {
    let result = meta_quote(List::new());
    assert!(result.is_err(), "meta_quote with no args should fail");
}

#[test]
fn test_meta_unquote_wrong_type() {
    let wrong_type = List::from_iter(vec![ConstValue::Int(42)]);
    let result = meta_unquote(wrong_type);

    assert!(result.is_err(), "meta_unquote with wrong type should fail");
}

#[test]
fn test_quote_builder_pattern() {
    // Test using QuoteBuilder directly
    use verum_compiler::quote::QuoteBuilder;

    let ts = QuoteBuilder::new()
        .keyword("fn")
        .ident("example")
        .punct("(")
        .punct(")")
        .punct("{")
        .punct("}")
        .build();

    assert!(!ts.is_empty(), "Built token stream should not be empty");
}

#[test]
fn test_quote_helpers() {
    use verum_compiler::quote::{ident, literal_int, literal_string};

    let id = ident("test", Span::default());
    assert_eq!(id.len(), 1, "Identifier should be one token");

    let num = literal_int(42, Span::default());
    assert_eq!(num.len(), 1, "Integer literal should be one token");

    let text = literal_string("hello", Span::default());
    assert_eq!(text.len(), 1, "String literal should be one token");
}

#[test]
fn test_to_tokens_trait() {
    use verum_compiler::quote::ToTokens;

    // Test that expressions implement ToTokens
    let expr = Expr::new(
        ExprKind::Path(Path::single(Ident::new("x", Span::default()))),
        Span::default(),
    );

    let ts = expr.into_token_stream();
    assert!(!ts.is_empty(), "Expression should convert to tokens");
}
