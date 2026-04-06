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
//! Tests for termination checking functionality
//!
//! These tests verify that the termination checker correctly identifies:
//! - Structurally recursive functions
//! - Non-terminating functions
//! - Mutual recursion cycles

use verum_ast::decl::{FunctionBody, FunctionDecl, FunctionParam, FunctionParamKind};
use verum_ast::expr::{Block, Expr, ExprKind};
use verum_ast::pattern::{Pattern, PatternKind};
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path, PathSegment, Type, TypeKind};
use verum_common::{List, Maybe, Text};
use verum_types::termination::TerminationChecker;

/// Helper to create a simple identifier
fn ident(name: &str) -> Ident {
    Ident {
        name: Text::from(name),
        span: Span::default(),
    }
}

/// Helper to create a simple path
fn path(name: &str) -> Path {
    Path {
        segments: smallvec::smallvec![PathSegment::Name(ident(name))],
        span: Span::default(),
    }
}

/// Helper to create a function parameter
fn param(name: &str) -> FunctionParam {
    FunctionParam::new(
        FunctionParamKind::Regular {
            pattern: Pattern {
                kind: PatternKind::Ident {
                    by_ref: false,
                    mutable: false,
                    name: ident(name),
                    subpattern: Maybe::None,
                },
                span: Span::default(),
            },
            ty: Type::new(TypeKind::Path(path("Int")), Span::default()),
            default_value: Maybe::None,
        },
        Span::default(),
    )
}

/// Helper to create a function call expression
fn call_expr(func_name: &str, args: Vec<Expr>) -> Expr {
    Expr {
        kind: ExprKind::Call {
            func: Box::new(Expr {
                kind: ExprKind::Path(path(func_name)),
                span: Span::default(),
                ref_kind: None,
                check_eliminated: false,
            }),
            args: args.into_iter().collect(),
            type_args: vec![].into(),
        },
        span: Span::default(),
        ref_kind: None,
        check_eliminated: false,
    }
}

/// Helper to create a path expression
fn path_expr(name: &str) -> Expr {
    Expr {
        kind: ExprKind::Path(path(name)),
        span: Span::default(),
        ref_kind: None,
        check_eliminated: false,
    }
}

#[test]
fn test_non_recursive_function() {
    let mut checker = TerminationChecker::new();

    // fn simple(x: Int) -> Int { x + 1 }
    let decl = FunctionDecl {
        visibility: verum_ast::decl::Visibility::Private,
        is_async: false,
        is_meta: false,
        stage_level: 0,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        is_variadic: false,
        extern_abi: Maybe::None,
        name: ident("simple"),
        generics: List::new(),
        params: vec![param("x")].into_iter().collect(),
        return_type: Maybe::Some(Type::new(TypeKind::Path(path("Int")), Span::default())),
        throws_clause: Maybe::None,
        std_attr: Maybe::None,
        contexts: List::new(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::Some(FunctionBody::Expr(path_expr("x"))),
        span: Span::default(),
    };

    // Non-recursive functions should pass termination checking
    assert!(checker.check_function(&decl).is_ok());
}

#[test]
fn test_simple_recursive_function() {
    let mut checker = TerminationChecker::new();

    // fn factorial(n: Int) -> Int {
    //     factorial(n)  // Non-terminating: no decreasing argument
    // }
    let body_expr = call_expr("factorial", vec![path_expr("n")]);

    let decl = FunctionDecl {
        visibility: verum_ast::decl::Visibility::Private,
        is_async: false,
        is_meta: false,
        stage_level: 0,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        is_variadic: false,
        extern_abi: Maybe::None,
        name: ident("factorial"),
        generics: List::new(),
        params: vec![param("n")].into_iter().collect(),
        return_type: Maybe::Some(Type::new(TypeKind::Path(path("Int")), Span::default())),
        throws_clause: Maybe::None,
        std_attr: Maybe::None,
        contexts: List::new(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::Some(FunctionBody::Expr(body_expr)),
        span: Span::default(),
    };

    // This should fail because there's no decreasing argument
    let result = checker.check_function(&decl);
    assert!(
        result.is_err(),
        "Expected termination error for non-decreasing recursion"
    );
}

#[test]
fn test_termination_checker_creation() {
    let _checker = TerminationChecker::new();
    // Verify the checker is initialized correctly
    // Note: internal state is private, so we just verify it constructs
}

#[test]
fn test_function_with_no_body() {
    let mut checker = TerminationChecker::new();

    // External function with no body
    let decl = FunctionDecl {
        visibility: verum_ast::decl::Visibility::Public,
        is_async: false,
        is_meta: false,
        stage_level: 0,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        is_variadic: false,
        extern_abi: Maybe::None,
        name: ident("external_func"),
        generics: List::new(),
        params: List::new(),
        return_type: Maybe::Some(Type::new(TypeKind::Path(path("Unit")), Span::default())),
        throws_clause: Maybe::None,
        std_attr: Maybe::None,
        contexts: List::new(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::None,
        span: Span::default(),
    };

    // Functions with no body should pass (they're external declarations)
    assert!(checker.check_function(&decl).is_ok());
}

#[test]
fn test_block_body_function() {
    let mut checker = TerminationChecker::new();

    // fn test() { }
    let empty_block = Block {
        stmts: List::new(),
        expr: Maybe::None,
        span: Span::default(),
    };

    let decl = FunctionDecl {
        visibility: verum_ast::decl::Visibility::Private,
        is_async: false,
        is_meta: false,
        stage_level: 0,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        is_variadic: false,
        extern_abi: Maybe::None,
        name: ident("test"),
        generics: List::new(),
        params: List::new(),
        return_type: Maybe::Some(Type::new(TypeKind::Path(path("Unit")), Span::default())),
        throws_clause: Maybe::None,
        std_attr: Maybe::None,
        contexts: List::new(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::Some(FunctionBody::Block(empty_block)),
        span: Span::default(),
    };

    // Empty function should pass
    assert!(checker.check_function(&decl).is_ok());
}
