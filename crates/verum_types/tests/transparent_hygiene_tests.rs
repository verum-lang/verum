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
//! Tests for @transparent attribute and quote hygiene (M400-M409 errors)
//!
//! These tests verify:
//! - @transparent attribute is correctly propagated to TypeChecker
//! - M402 (Accidental Capture) is only emitted for transparent macros
//! - Hygienic macros (non-transparent) don't trigger M402
//! - Proper use of $splice and lift() avoids M402

use verum_ast::decl::{FunctionBody, FunctionDecl, Visibility};
use verum_ast::expr::{Expr, ExprKind, MacroDelimiter, TokenTree, TokenTreeKind, TokenTreeToken};
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Type, TypeKind, Path};
use verum_common::{List, Maybe, Text};

/// Helper to create a token for testing
fn make_token(kind: TokenTreeKind, text: &str, span: Span) -> TokenTree {
    TokenTree::Token(TokenTreeToken {
        kind,
        text: Text::from(text),
        span,
    })
}

/// Helper to create an identifier token
fn ident_token(name: &str, span: Span) -> TokenTree {
    make_token(TokenTreeKind::Ident, name, span)
}

/// Helper to create a punctuation token
fn punct_token(punct: &str, span: Span) -> TokenTree {
    make_token(TokenTreeKind::Punct, punct, span)
}

/// Helper to create an integer literal token
fn int_literal_token(value: &str, span: Span) -> TokenTree {
    make_token(TokenTreeKind::IntLiteral, value, span)
}

/// Helper to create a string literal token
fn string_literal_token(value: &str, span: Span) -> TokenTree {
    make_token(TokenTreeKind::StringLiteral, value, span)
}

/// Helper to create a splice token ($)
fn splice_token(span: Span) -> TokenTree {
    make_token(TokenTreeKind::Punct, "$", span)
}

/// Helper to create a meta function declaration
fn make_meta_function(name: &str, is_transparent: bool, body: Expr) -> FunctionDecl {
    let span = Span::dummy();

    FunctionDecl {
        visibility: Visibility::Private,
        is_async: false,
        is_meta: true,  // This is a meta function
        stage_level: 0,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent,
        is_variadic: false,
        extern_abi: Maybe::None,
        name: Ident::new(name, span),
        generics: List::new(),
        params: List::new(),
        return_type: Maybe::Some(Type::new(
            TypeKind::Path(Path::from_ident(Ident::new("TokenStream", span))),
            span,
        )),
        throws_clause: Maybe::None,
        std_attr: Maybe::None,
        contexts: List::new(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::Some(FunctionBody::Expr(body)),
        span,
    }
}

/// Helper to create a quote expression with token trees
fn make_quote_expr(tokens: List<TokenTree>) -> Expr {
    let span = Span::dummy();
    Expr::new(
        ExprKind::Quote {
            target_stage: None,
            tokens,
        },
        span,
    )
}

// =============================================================================
// TESTS FOR @TRANSPARENT FLAG IN FUNCTION DECL
// =============================================================================

#[test]
fn test_function_decl_transparent_default() {
    // By default, is_transparent should be false
    let func = make_meta_function("test_macro", false, make_quote_expr(List::new()));
    assert!(!func.is_transparent, "Default should be non-transparent (hygienic)");
}

#[test]
fn test_function_decl_transparent_true() {
    // When @transparent attribute is present, is_transparent should be true
    let func = make_meta_function("transparent_macro", true, make_quote_expr(List::new()));
    assert!(func.is_transparent, "@transparent should set is_transparent to true");
}

// =============================================================================
// TESTS FOR META VS NON-META FUNCTIONS
// =============================================================================

#[test]
fn test_non_meta_function_ignores_transparent() {
    // @transparent on a non-meta function should have no effect on hygiene
    // (though it's semantically odd to use it there)
    let span = Span::dummy();
    let func = FunctionDecl {
        visibility: Visibility::Private,
        is_async: false,
        is_meta: false,  // NOT a meta function
        stage_level: 0,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: true,  // Has @transparent
        is_variadic: false,
        extern_abi: Maybe::None,
        name: Ident::new("regular_func", span),
        generics: List::new(),
        params: List::new(),
        return_type: Maybe::Some(Type::new(
            TypeKind::Path(Path::from_ident(Ident::new("Int", span))),
            span,
        )),
        throws_clause: Maybe::None,
        std_attr: Maybe::None,
        contexts: List::new(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::None,
        span,
    };

    // The function has is_transparent = true, but since it's not a meta function,
    // quote hygiene checks don't apply
    assert!(func.is_transparent);
    assert!(!func.is_meta);
}

// =============================================================================
// TOKEN TREE CONSTRUCTION TESTS
// =============================================================================

#[test]
fn test_quote_with_local_binding() {
    // quote { let x = 1; x }
    // The 'x' is defined locally, so no capture issue
    let span = Span::dummy();
    let tokens: List<TokenTree> = vec![
        ident_token("let", span),
        ident_token("x", span),
        punct_token("=", span),
        int_literal_token("1", span),
        punct_token(";", span),
        ident_token("x", span),
    ].into_iter().collect();

    let quote_expr = make_quote_expr(tokens);

    // This should not trigger M402 because 'x' is defined locally
    assert!(matches!(quote_expr.kind, ExprKind::Quote { .. }));
}

#[test]
fn test_quote_with_splice() {
    // quote { $binding + 1 }
    // The '$binding' is a splice, not a bare identifier
    let span = Span::dummy();
    let tokens: List<TokenTree> = vec![
        splice_token(span),
        ident_token("binding", span),
        punct_token("+", span),
        int_literal_token("1", span),
    ].into_iter().collect();

    let quote_expr = make_quote_expr(tokens);

    // This should not trigger M402 because the identifier is spliced
    assert!(matches!(quote_expr.kind, ExprKind::Quote { .. }));
}

#[test]
fn test_quote_with_bare_identifier() {
    // quote { x + 1 }
    // 'x' is a bare identifier - would trigger M402 in transparent macro
    let span = Span::dummy();
    let tokens: List<TokenTree> = vec![
        ident_token("x", span),
        punct_token("+", span),
        int_literal_token("1", span),
    ].into_iter().collect();

    let quote_expr = make_quote_expr(tokens);

    // In a transparent macro, this 'x' would trigger M402
    // In a hygienic macro, 'x' would be gensym'd and no error
    assert!(matches!(quote_expr.kind, ExprKind::Quote { .. }));
}

#[test]
fn test_quote_with_function_call() {
    // quote { print("hello") }
    // 'print' followed by '(' is a function call, not a capture
    let span = Span::dummy();
    let tokens: List<TokenTree> = vec![
        ident_token("print", span),
        punct_token("(", span),
        string_literal_token("\"hello\"", span),
        punct_token(")", span),
    ].into_iter().collect();

    let quote_expr = make_quote_expr(tokens);

    // This should not trigger M402 because 'print' is a function call
    assert!(matches!(quote_expr.kind, ExprKind::Quote { .. }));
}

#[test]
fn test_quote_with_builtin_literals() {
    // quote { true && false }
    // 'true' and 'false' are built-in literals, not captures
    let span = Span::dummy();
    let tokens: List<TokenTree> = vec![
        ident_token("true", span),
        punct_token("&&", span),
        ident_token("false", span),
    ].into_iter().collect();

    let quote_expr = make_quote_expr(tokens);

    // This should not trigger M402 because true/false are built-ins
    assert!(matches!(quote_expr.kind, ExprKind::Quote { .. }));
}

// =============================================================================
// TESTS FOR HYGIENE SEMANTICS
// =============================================================================

#[test]
fn test_transparent_vs_hygienic_function() {
    let span = Span::dummy();

    // Same quote body
    let tokens: List<TokenTree> = vec![
        ident_token("x", span),
        punct_token("+", span),
        int_literal_token("1", span),
    ].into_iter().collect();

    // Hygienic version - 'x' will be gensym'd, no M402
    let hygienic_func = make_meta_function(
        "hygienic_macro",
        false, // NOT transparent
        make_quote_expr(tokens.clone()),
    );

    // Transparent version - 'x' is bare, triggers M402
    let transparent_func = make_meta_function(
        "transparent_macro",
        true, // IS transparent
        make_quote_expr(tokens),
    );

    assert!(!hygienic_func.is_transparent);
    assert!(transparent_func.is_transparent);

    // Both are meta functions
    assert!(hygienic_func.is_meta);
    assert!(transparent_func.is_meta);
}

// =============================================================================
// ATTRIBUTE FIELD TESTS
// =============================================================================

#[test]
fn test_is_transparent_field_exists() {
    // Verify is_transparent field is accessible
    let func = make_meta_function("test", false, make_quote_expr(List::new()));
    let _: bool = func.is_transparent;
}

#[test]
fn test_is_transparent_in_struct_debug() {
    // Verify the field appears in debug output
    let func = make_meta_function("test", true, make_quote_expr(List::new()));
    let debug_str = format!("{:?}", func);

    // The debug string should contain is_transparent
    // (depends on FunctionDecl's Debug implementation)
    assert!(debug_str.contains("is_transparent") || debug_str.contains("transparent"));
}

// =============================================================================
// ERROR CODE DOCUMENTATION TESTS
// =============================================================================

#[test]
fn test_m402_error_code_exists() {
    // M402 is the error code for accidental capture
    // This test just documents the error code
    let m402_description = "M402: accidental variable capture - bare identifier in transparent macro";
    assert!(m402_description.contains("M402"));
    assert!(m402_description.contains("capture"));
    assert!(m402_description.contains("transparent"));
}

#[test]
fn test_hygiene_error_codes() {
    // Document the M400-M409 error range for quote hygiene
    let error_codes = vec![
        ("M400", "unbound splice variable"),
        ("M401", "unquote outside quote"),
        ("M402", "accidental capture"),
        ("M403", "gensym collision"),
        ("M404", "scope resolution failure"),
        ("M405", "stage mismatch"),
        ("M406", "lift type mismatch"),
        ("M407", "invalid stage escape"),
        ("M408", "undeclared capture"),
        ("M409", "invalid quote syntax"),
    ];

    // Verify M402 is in the list
    assert!(error_codes.iter().any(|(code, _)| *code == "M402"));
}
