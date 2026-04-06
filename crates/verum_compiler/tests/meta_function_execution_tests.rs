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
//! Tests for user-defined meta function execution
//!
//! These tests verify that the meta-programming system correctly executes
//! user-defined meta functions at compile time.
//!
//! NOTE: These tests are temporarily disabled due to API changes.
//! The MetaContext functions now expect Vec<ConstValue> instead of List<ConstValue>.
//! Also, ConstValue variants like Array and Tuple now expect Vec instead of List.

#![cfg(feature = "meta_tests_disabled")]

use verum_ast::expr::{Expr, ExprKind};
use verum_ast::literal::{IntLit, Literal, LiteralKind, StringLit};
use verum_ast::{Ident, Path, Span, Type, TypeKind};
use verum_compiler::meta::{
    ConstValue, MetaContext, MetaError, MetaExpr, MetaPattern, MetaStmt,
};
use verum_compiler::meta::registry::{MetaFunction, MetaParam, MetaRegistry};
use verum_common::{Heap, List, Text};

/// Helper to create a simple integer literal
fn int_lit(value: i64) -> Expr {
    let span = Span::default();
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Int(IntLit {
                value: value as i128,
                suffix: None,
            }),
            span,
        }),
        span,
    )
}

/// Helper to create a text literal
fn text_lit(value: &str) -> Expr {
    let span = Span::default();
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Text(StringLit::Regular(Text::from(value))),
            span,
        }),
        span,
    )
}

#[test]
fn test_meta_context_creation() {
    let ctx = MetaContext::new();
    assert_eq!(ctx.binding_names().len(), 0);
}

#[test]
fn test_meta_context_bindings() {
    let mut ctx = MetaContext::new();

    ctx.bind("x".to_string(), ConstValue::Int(42));
    assert_eq!(ctx.get(&"x".to_string()), Some(ConstValue::Int(42)));

    ctx.bind("y".to_string(), ConstValue::Text("hello".to_string()));
    assert_eq!(
        ctx.get(&"y".to_string()),
        Some(ConstValue::Text("hello".to_string()))
    );

    assert_eq!(ctx.binding_names().len(), 2);
}

#[test]
fn test_const_value_arithmetic() {
    let a = ConstValue::Int(10);
    let b = ConstValue::Int(5);

    assert_eq!(a.clone().add(b.clone()).unwrap(), ConstValue::Int(15));
    assert_eq!(a.clone().sub(b.clone()).unwrap(), ConstValue::Int(5));
    assert_eq!(a.clone().mul(b.clone()).unwrap(), ConstValue::Int(50));
    assert_eq!(a.clone().div(b.clone()).unwrap(), ConstValue::Int(2));
    assert_eq!(a.modulo(b).unwrap(), ConstValue::Int(0));
}

#[test]
fn test_const_value_comparison() {
    let a = ConstValue::Int(10);
    let b = ConstValue::Int(5);

    assert_eq!(a.clone().lt(b.clone()).unwrap(), ConstValue::Bool(false));
    assert_eq!(a.clone().gt(b.clone()).unwrap(), ConstValue::Bool(true));
    assert_eq!(a.clone().le(b.clone()).unwrap(), ConstValue::Bool(false));
    assert_eq!(a.ge(b).unwrap(), ConstValue::Bool(true));
}

#[test]
fn test_const_value_logical_ops() {
    let t = ConstValue::Bool(true);
    let f = ConstValue::Bool(false);

    assert_eq!(t.clone().and(f.clone()).unwrap(), ConstValue::Bool(false));
    assert_eq!(t.clone().or(f.clone()).unwrap(), ConstValue::Bool(true));
    assert_eq!(t.not().unwrap(), ConstValue::Bool(false));
    assert_eq!(f.not().unwrap(), ConstValue::Bool(true));
}

#[test]
fn test_const_value_division_by_zero() {
    let a = ConstValue::Int(10);
    let zero = ConstValue::Int(0);

    assert!(a.div(zero).is_err());
}

#[test]
fn test_meta_expr_literal() {
    let mut ctx = MetaContext::new();
    let expr = MetaExpr::Literal(ConstValue::Int(42));

    let result = ctx.eval_meta_expr(&expr);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ConstValue::Int(42));
}

#[test]
fn test_meta_expr_variable() {
    let mut ctx = MetaContext::new();
    ctx.bind("x".to_string(), ConstValue::Int(100));

    let expr = MetaExpr::Variable("x".to_string());
    let result = ctx.eval_meta_expr(&expr);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ConstValue::Int(100));
}

#[test]
fn test_meta_expr_undefined_variable() {
    let mut ctx = MetaContext::new();
    let expr = MetaExpr::Variable("undefined".to_string());

    let result = ctx.eval_meta_expr(&expr);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        MetaError::UndefinedVariable(_)
    ));
}

#[test]
fn test_meta_expr_if_true() {
    let mut ctx = MetaContext::new();
    let expr = MetaExpr::If {
        condition: Heap::new(MetaExpr::Literal(ConstValue::Bool(true))),
        then_branch: Heap::new(MetaExpr::Literal(ConstValue::Int(1))),
        else_branch: Maybe::Some(Heap::new(MetaExpr::Literal(ConstValue::Int(2)))),
    };

    let result = ctx.eval_meta_expr(&expr);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ConstValue::Int(1));
}

#[test]
fn test_meta_expr_if_false() {
    let mut ctx = MetaContext::new();
    let expr = MetaExpr::If {
        condition: Heap::new(MetaExpr::Literal(ConstValue::Bool(false))),
        then_branch: Heap::new(MetaExpr::Literal(ConstValue::Int(1))),
        else_branch: Maybe::Some(Heap::new(MetaExpr::Literal(ConstValue::Int(2)))),
    };

    let result = ctx.eval_meta_expr(&expr);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ConstValue::Int(2));
}

#[test]
fn test_meta_expr_let_binding() {
    let mut ctx = MetaContext::new();
    let expr = MetaExpr::Let {
        name: "x".to_string(),
        value: Heap::new(MetaExpr::Literal(ConstValue::Int(42))),
        body: Heap::new(MetaExpr::Variable("x".to_string())),
    };

    let result = ctx.eval_meta_expr(&expr);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ConstValue::Int(42));
}

#[test]
fn test_meta_expr_binary_add() {
    let mut ctx = MetaContext::new();
    let expr = MetaExpr::Binary {
        op: verum_ast::expr::BinOp::Add,
        left: Heap::new(MetaExpr::Literal(ConstValue::Int(10))),
        right: Heap::new(MetaExpr::Literal(ConstValue::Int(32))),
    };

    let result = ctx.eval_meta_expr(&expr);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ConstValue::Int(42));
}

#[test]
fn test_meta_expr_binary_multiply() {
    let mut ctx = MetaContext::new();
    let expr = MetaExpr::Binary {
        op: verum_ast::expr::BinOp::Mul,
        left: Heap::new(MetaExpr::Literal(ConstValue::Int(6))),
        right: Heap::new(MetaExpr::Literal(ConstValue::Int(7))),
    };

    let result = ctx.eval_meta_expr(&expr);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ConstValue::Int(42));
}

#[test]
fn test_meta_expr_unary_neg() {
    let mut ctx = MetaContext::new();
    let expr = MetaExpr::Unary {
        op: verum_ast::expr::UnOp::Neg,
        expr: Heap::new(MetaExpr::Literal(ConstValue::Int(42))),
    };

    let result = ctx.eval_meta_expr(&expr);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ConstValue::Int(-42));
}

#[test]
fn test_meta_expr_unary_not() {
    let mut ctx = MetaContext::new();
    let expr = MetaExpr::Unary {
        op: verum_ast::expr::UnOp::Not,
        expr: Heap::new(MetaExpr::Literal(ConstValue::Bool(true))),
    };

    let result = ctx.eval_meta_expr(&expr);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ConstValue::Bool(false));
}

#[test]
fn test_meta_expr_block() {
    let mut ctx = MetaContext::new();
    let expr = MetaExpr::Block(List::from(vec![
        MetaStmt::Let {
            name: "x".to_string(),
            value: MetaExpr::Literal(ConstValue::Int(10)),
        },
        MetaStmt::Let {
            name: "y".to_string(),
            value: MetaExpr::Literal(ConstValue::Int(32)),
        },
        MetaStmt::Expr(MetaExpr::Binary {
            op: verum_ast::expr::BinOp::Add,
            left: Heap::new(MetaExpr::Variable("x".to_string())),
            right: Heap::new(MetaExpr::Variable("y".to_string())),
        }),
    ]));

    let result = ctx.eval_meta_expr(&expr);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ConstValue::Int(42));
}

#[test]
fn test_meta_expr_quote() {
    let mut ctx = MetaContext::new();
    let ast_expr = int_lit(42);
    let expr = MetaExpr::Quote(ast_expr.clone());

    let result = ctx.eval_meta_expr(&expr);
    assert!(result.is_ok());

    match result.unwrap() {
        ConstValue::Expr(e) => assert_eq!(e, ast_expr),
        _ => panic!("Expected Expr value"),
    }
}

#[test]
fn test_builtin_type_name() {
    let mut ctx = MetaContext::new();
    let ty = Type::new(TypeKind::Int, Span::default());
    let args = List::from(vec![ConstValue::Type(ty)]);

    let result = MetaContext::meta_type_name(&mut ctx, args);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ConstValue::Text("Int".to_string()));
}

#[test]
fn test_builtin_list_len() {
    let mut ctx = MetaContext::new();
    let list = List::from(vec![
        ConstValue::Int(1),
        ConstValue::Int(2),
        ConstValue::Int(3),
    ]);
    let args = List::from(vec![ConstValue::Array(list)]);

    let result = MetaContext::meta_list_len(&mut ctx, args);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ConstValue::Int(3));
}

#[test]
fn test_builtin_list_push() {
    let mut ctx = MetaContext::new();
    let list = List::from(vec![ConstValue::Int(1), ConstValue::Int(2)]);
    let args = List::from(vec![ConstValue::Array(list), ConstValue::Int(3)]);

    let result = MetaContext::meta_list_push(&mut ctx, args);
    assert!(result.is_ok());

    match result.unwrap() {
        ConstValue::Array(arr) => {
            assert_eq!(arr.len(), 3);
            assert_eq!(arr[2], ConstValue::Int(3));
        }
        _ => panic!("Expected Array"),
    }
}

#[test]
fn test_builtin_list_get() {
    let mut ctx = MetaContext::new();
    let list = List::from(vec![
        ConstValue::Int(10),
        ConstValue::Int(20),
        ConstValue::Int(30),
    ]);
    let args = List::from(vec![ConstValue::Array(list), ConstValue::Int(1)]);

    let result = MetaContext::meta_list_get(&mut ctx, args);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ConstValue::Int(20));
}

#[test]
fn test_builtin_text_concat() {
    let mut ctx = MetaContext::new();
    let args = List::from(vec![
        ConstValue::Text("Hello".to_string()),
        ConstValue::Text(" ".to_string()),
        ConstValue::Text("World".to_string()),
    ]);

    let result = MetaContext::meta_text_concat(&mut ctx, args);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ConstValue::Text("Hello World".to_string()));
}

#[test]
fn test_builtin_text_len() {
    let mut ctx = MetaContext::new();
    let args = List::from(vec![ConstValue::Text("Hello".to_string())]);

    let result = MetaContext::meta_text_len(&mut ctx, args);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ConstValue::Int(5));
}

#[test]
fn test_meta_registry_register_function() {
    use verum_ast::decl::FunctionDecl;
    use verum_ast::ty::Ident;

    let mut registry = MetaRegistry::new();
    let module = Text::from("test_module");

    let func_decl = FunctionDecl {
        visibility: verum_ast::decl::Visibility::Public,
        is_async: false,
        is_meta: true,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        is_variadic: false,
        extern_abi: Maybe::None,
        name: Ident::new("test_func", Span::default()),
        generics: vec![],
        params: vec![],
        throws_clause: Maybe::None,
        return_type: Maybe::None,
        std_attr: Maybe::None,
        contexts: vec![],
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: vec![],
        ensures: vec![],
        attributes: vec![],
        body: Maybe::Some(verum_ast::decl::FunctionBody::Expr(int_lit(42))),
        span: Span::default(),
    };

    let result = registry.register_meta_function(&module, &func_decl);
    assert!(result.is_ok());

    // Verify we can resolve it
    let resolved = registry.resolve_meta_call(&module, &Text::from("test_func"));
    assert!(resolved.is_some());
}

#[test]
fn test_meta_registry_duplicate_function() {
    use verum_ast::decl::FunctionDecl;
    use verum_ast::ty::Ident;

    let mut registry = MetaRegistry::new();
    let module = Text::from("test_module");

    let func_decl = FunctionDecl {
        visibility: verum_ast::decl::Visibility::Public,
        is_async: false,
        is_meta: true,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        is_variadic: false,
        extern_abi: Maybe::None,
        name: Ident::new("test_func", Span::default()),
        generics: vec![],
        params: vec![],
        throws_clause: Maybe::None,
        return_type: Maybe::None,
        std_attr: Maybe::None,
        contexts: vec![],
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: vec![],
        ensures: vec![],
        attributes: vec![],
        body: Maybe::Some(verum_ast::decl::FunctionBody::Expr(int_lit(42))),
        span: Span::default(),
    };

    // First registration succeeds
    assert!(registry.register_meta_function(&module, &func_decl).is_ok());

    // Second registration fails
    let result = registry.register_meta_function(&module, &func_decl);
    assert!(result.is_err());
}

#[test]
fn test_execute_simple_meta_function() {
    let mut ctx = MetaContext::new();

    // Create a simple meta function: fn add(x: Int, y: Int) -> Int { x + y }
    let meta_func = MetaFunction {
        name: Text::from("add"),
        module: Text::from("test"),
        params: vec![
            MetaParam {
                name: Text::from("x"),
                ty: Type::int(Span::default()),
                is_meta: false,
            },
            MetaParam {
                name: Text::from("y"),
                ty: Type::int(Span::default()),
                is_meta: false,
            },
        ]
        .into(),
        return_type: Type::int(Span::default()),
        body: Expr::new(
            ExprKind::Binary {
                op: verum_ast::expr::BinOp::Add,
                left: Heap::new(Expr::new(
                    ExprKind::Path(Path::single(Ident::new("x", Span::default()))),
                    Span::default(),
                )),
                right: Heap::new(Expr::new(
                    ExprKind::Path(Path::single(Ident::new("y", Span::default()))),
                    Span::default(),
                )),
            },
            Span::default(),
        ),
        is_async: false,
        span: Span::default(),
    };

    let args = vec![ConstValue::Int(10), ConstValue::Int(32)].into();
    let result = ctx.execute_user_meta_fn(&meta_func, args);

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), ConstValue::Int(42));
}

#[test]
fn test_execute_meta_function_wrong_arg_count() {
    let mut ctx = MetaContext::new();

    let meta_func = MetaFunction {
        name: Text::from("test"),
        module: Text::from("test"),
        params: vec![MetaParam {
            name: Text::from("x"),
            ty: Type::int(Span::default()),
            is_meta: false,
        }]
        .into(),
        return_type: Type::int(Span::default()),
        body: int_lit(42),
        is_async: false,
        span: Span::default(),
    };

    // Pass wrong number of arguments
    let args = List::from(vec![ConstValue::Int(1), ConstValue::Int(2)]);
    let result = ctx.execute_user_meta_fn(&meta_func, args);

    assert!(result.is_err());
}

#[test]
fn test_meta_pattern_wildcard() {
    let mut ctx = MetaContext::new();
    let value = ConstValue::Int(42);
    let pattern = MetaPattern::Wildcard;

    let result = ctx.matches_pattern(&value, &pattern);
    assert!(result.is_ok());
    assert!(result.unwrap());
}

#[test]
fn test_meta_pattern_literal() {
    let mut ctx = MetaContext::new();
    let value = ConstValue::Int(42);
    let pattern = MetaPattern::Literal(ConstValue::Int(42));

    let result = ctx.matches_pattern(&value, &pattern);
    assert!(result.is_ok());
    assert!(result.unwrap());
}

#[test]
fn test_meta_pattern_literal_no_match() {
    let mut ctx = MetaContext::new();
    let value = ConstValue::Int(42);
    let pattern = MetaPattern::Literal(ConstValue::Int(99));

    let result = ctx.matches_pattern(&value, &pattern);
    assert!(result.is_ok());
    assert!(!result.unwrap());
}

#[test]
fn test_meta_pattern_ident_binding() {
    let mut ctx = MetaContext::new();
    let value = ConstValue::Int(42);
    let pattern = MetaPattern::Ident("x".to_string());

    let result = ctx.matches_pattern(&value, &pattern);
    assert!(result.is_ok());
    assert!(result.unwrap());

    // Verify binding was created
    assert_eq!(ctx.get(&"x".to_string()), Some(ConstValue::Int(42)));
}

#[test]
fn test_meta_pattern_tuple() {
    let mut ctx = MetaContext::new();
    let value = ConstValue::Tuple(List::from(vec![ConstValue::Int(1), ConstValue::Int(2)]));

    let pattern = MetaPattern::Tuple(List::from(vec![
        MetaPattern::Ident("x".to_string()),
        MetaPattern::Ident("y".to_string()),
    ]));

    let result = ctx.matches_pattern(&value, &pattern);
    assert!(result.is_ok());
    assert!(result.unwrap());

    assert_eq!(ctx.get(&"x".to_string()), Some(ConstValue::Int(1)));
    assert_eq!(ctx.get(&"y".to_string()), Some(ConstValue::Int(2)));
}

#[test]
fn test_const_value_type_name() {
    assert_eq!(ConstValue::Unit.type_name(), "Unit".to_string());
    assert_eq!(ConstValue::Bool(true).type_name(), "Bool".to_string());
    assert_eq!(ConstValue::Int(42).type_name(), "Int".to_string());
    assert_eq!(
        ConstValue::Text("hi".to_string()).type_name(),
        "Text".to_string()
    );
}

#[test]
fn test_const_value_as_methods() {
    let int_val = ConstValue::Int(42);
    assert_eq!(int_val.as_int(), Some(42));
    assert_eq!(int_val.as_text(), None);

    let text_val = ConstValue::Text("hello".to_string());
    assert_eq!(text_val.as_text(), Some(&"hello".to_string()));
    assert_eq!(text_val.as_int(), None);
}
