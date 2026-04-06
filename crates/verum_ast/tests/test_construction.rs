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
    unused_assignments,
    clippy::approx_constant
)]
//! Tests for constructing all AST node types.
//!
//! This module ensures that all AST node constructors work correctly
//! and produce valid nodes with expected properties.

use verum_ast::literal::*;
use verum_ast::*;
use verum_common::List;
use verum_common::{Heap, Maybe};

/// Helper function to create a test span
fn test_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

/// Helper function to create a test identifier
fn test_ident(name: &str) -> Ident {
    Ident::new(name.to_string(), test_span())
}

#[test]
fn test_module_construction() {
    let file_id = FileId::new(0);
    let span = test_span();

    // Test empty module
    let empty = Module::empty(file_id);
    assert_eq!(empty.items.len(), 0);
    assert_eq!(empty.file_id, file_id);

    // Test module with items
    let items = vec![Item::new(
        ItemKind::Mount(MountDecl {
            visibility: Visibility::Private,
            tree: MountTree { alias: Maybe::None,
                kind: MountTreeKind::Path(Path::single(test_ident("std"))),
                span,
            },
            alias: Maybe::None,
            span,
        }),
        span,
    )];
    let module = Module::new(List::from(items.clone()), file_id, span);
    assert_eq!(module.items.len(), 1);
    assert_eq!(module.file_id, file_id);
    assert_eq!(module.span, span);
}

#[test]
fn test_compilation_unit_construction() {
    let module1 = Module::empty(FileId::new(0));
    let module2 = Module::empty(FileId::new(1));

    // Test single module
    let single = CompilationUnit::single(module1.clone());
    assert_eq!(single.modules.len(), 1);

    // Test multiple modules
    let multi = CompilationUnit::new(vec![module1, module2].into());
    assert_eq!(multi.modules.len(), 2);
}

#[test]
fn test_literal_construction() {
    let span = test_span();

    // Test integer literals
    let int_lit = Literal::int(42, span);
    assert!(matches!(int_lit.kind, LiteralKind::Int(ref i) if i.value == 42));
    assert_eq!(int_lit.span, span);

    // Test float literals
    let float_lit = Literal::float(3.14, span);
    assert!(matches!(float_lit.kind, LiteralKind::Float(ref f) if f.value == 3.14));

    // Test string literals
    let str_lit = Literal::string("hello".to_string().into(), span);
    assert!(matches!(str_lit.kind, LiteralKind::Text(StringLit::Regular(ref s)) if s == "hello"));

    // Test boolean literals
    let bool_true = Literal::bool(true, span);
    assert_eq!(bool_true.kind, LiteralKind::Bool(true));

    let bool_false = Literal::bool(false, span);
    assert_eq!(bool_false.kind, LiteralKind::Bool(false));

    // Test char literal
    let char_lit = Literal::char('a', span);
    assert_eq!(char_lit.kind, LiteralKind::Char('a'));
}

#[test]
fn test_expression_construction() {
    let span = test_span();

    // Test literal expression
    let lit_expr = Expr::literal(Literal::int(42, span));
    assert!(matches!(lit_expr.kind, ExprKind::Literal(_)));

    // Test path expression
    let path = Path::single(test_ident("x"));
    let path_expr = Expr::path(path);
    assert!(matches!(path_expr.kind, ExprKind::Path(_)));

    // Test identifier expression
    let ident_expr = Expr::ident(test_ident("y"));
    assert!(matches!(ident_expr.kind, ExprKind::Path(_)));

    // Test binary expression
    let left = Heap::new(Expr::literal(Literal::int(1, span)));
    let right = Heap::new(Expr::literal(Literal::int(2, span)));
    let bin_expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left,
            right,
        },
        span,
    );
    assert!(matches!(bin_expr.kind, ExprKind::Binary { .. }));
}

#[test]
fn test_binary_operators() {
    let span = test_span();
    let left = Heap::new(Expr::literal(Literal::int(1, span)));
    let right = Heap::new(Expr::literal(Literal::int(2, span)));

    // Test all arithmetic operators
    let ops = vec![
        BinOp::Add,
        BinOp::Sub,
        BinOp::Mul,
        BinOp::Div,
        BinOp::Rem,
        BinOp::Pow,
    ];

    for op in ops {
        let expr = Expr::new(
            ExprKind::Binary {
                op,
                left: left.clone(),
                right: right.clone(),
            },
            span,
        );

        match &expr.kind {
            ExprKind::Binary { op: op_ref, .. } => assert_eq!(*op_ref, op),
            _ => panic!("Expected binary expression"),
        }
    }

    // Test logical operators
    let bool_left = Heap::new(Expr::literal(Literal::bool(true, span)));
    let bool_right = Heap::new(Expr::literal(Literal::bool(false, span)));

    let logical_ops = vec![BinOp::And, BinOp::Or];

    for op in logical_ops {
        let expr = Expr::new(
            ExprKind::Binary {
                op,
                left: bool_left.clone(),
                right: bool_right.clone(),
            },
            span,
        );
        assert!(matches!(expr.kind, ExprKind::Binary { .. }));
    }
}

#[test]
fn test_unary_operators() {
    let span = test_span();

    // Test negation
    let num_expr = Heap::new(Expr::literal(Literal::int(42, span)));
    let neg_expr = Expr::new(
        ExprKind::Unary {
            op: UnOp::Neg,
            expr: num_expr,
        },
        span,
    );
    assert!(matches!(
        neg_expr.kind,
        ExprKind::Unary { op: UnOp::Neg, .. }
    ));

    // Test logical not
    let bool_expr = Heap::new(Expr::literal(Literal::bool(true, span)));
    let not_expr = Expr::new(
        ExprKind::Unary {
            op: UnOp::Not,
            expr: bool_expr,
        },
        span,
    );
    assert!(matches!(
        not_expr.kind,
        ExprKind::Unary { op: UnOp::Not, .. }
    ));

    // Test dereference
    let ptr_expr = Heap::new(Expr::ident(test_ident("ptr")));
    let deref_expr = Expr::new(
        ExprKind::Unary {
            op: UnOp::Deref,
            expr: ptr_expr,
        },
        span,
    );
    assert!(matches!(
        deref_expr.kind,
        ExprKind::Unary {
            op: UnOp::Deref,
            ..
        }
    ));
}

#[test]
fn test_type_construction() {
    let span = test_span();

    // Test primitive types
    let unit_ty = Type::unit(span);
    assert_eq!(unit_ty.kind, TypeKind::Unit);
    assert_eq!(unit_ty.span, span);

    let bool_ty = Type::bool(span);
    assert_eq!(bool_ty.kind, TypeKind::Bool);

    let int_ty = Type::int(span);
    assert_eq!(int_ty.kind, TypeKind::Int);

    let float_ty = Type::float(span);
    assert_eq!(float_ty.kind, TypeKind::Float);

    let text_ty = Type::text(span);
    assert_eq!(text_ty.kind, TypeKind::Text);

    // Test inferred type
    let inferred_ty = Type::inferred(span);
    assert_eq!(inferred_ty.kind, TypeKind::Inferred);
}

#[test]
fn test_tuple_type_construction() {
    let span = test_span();

    // Empty tuple (unit)
    let empty_tuple = Type::new(TypeKind::Tuple(List::from(vec![])), span);
    assert!(matches!(empty_tuple.kind, TypeKind::Tuple(ref v) if v.is_empty()));

    // Single element tuple
    let single = Type::new(TypeKind::Tuple(List::from(vec![Type::int(span)])), span);
    assert!(matches!(single.kind, TypeKind::Tuple(ref v) if v.len() == 1));

    // Multiple element tuple
    let multi = Type::new(
        TypeKind::Tuple(List::from(vec![
            Type::int(span),
            Type::text(span),
            Type::bool(span),
        ])),
        span,
    );
    assert!(matches!(multi.kind, TypeKind::Tuple(ref v) if v.len() == 3));
}

#[test]
fn test_pattern_construction() {
    let span = test_span();

    // Test wildcard pattern
    let wildcard = Pattern::wildcard(span);
    assert_eq!(wildcard.kind, PatternKind::Wildcard);
    assert_eq!(wildcard.span, span);

    // Test identifier pattern (immutable)
    let ident = Pattern::ident(test_ident("x"), false, span);
    assert!(matches!(
        ident.kind,
        PatternKind::Ident { mutable: false, .. }
    ));

    // Test mutable identifier pattern
    let mut_ident = Pattern::ident(test_ident("y"), true, span);
    assert!(matches!(
        mut_ident.kind,
        PatternKind::Ident { mutable: true, .. }
    ));

    // Test literal pattern
    let lit_pattern = Pattern::literal(Literal::int(42, span));
    assert!(matches!(lit_pattern.kind, PatternKind::Literal(_)));
}

#[test]
fn test_tuple_pattern_construction() {
    let span = test_span();

    // Empty tuple pattern
    let empty = Pattern::new(PatternKind::Tuple(List::from(vec![])), span);
    assert!(matches!(empty.kind, PatternKind::Tuple(ref v) if v.is_empty()));

    // Multiple element tuple pattern
    let patterns = vec![
        Pattern::wildcard(span),
        Pattern::ident(test_ident("x"), false, span),
        Pattern::literal(Literal::int(42, span)),
    ];
    let tuple_pat = Pattern::new(PatternKind::Tuple(List::from(patterns)), span);
    assert!(matches!(tuple_pat.kind, PatternKind::Tuple(ref v) if v.len() == 3));
}

#[test]
fn test_slice_pattern_construction() {
    let span = test_span();

    // Simple slice pattern [a, b, c]
    let simple_slice = Pattern::new(
        PatternKind::Slice {
            before: List::from(vec![
                Pattern::ident(test_ident("a"), false, span),
                Pattern::ident(test_ident("b"), false, span),
                Pattern::ident(test_ident("c"), false, span),
            ]),
            rest: Maybe::None,
            after: List::from(vec![]),
        },
        span,
    );
    assert!(
        matches!(simple_slice.kind, PatternKind::Slice { ref before, .. } if before.len() == 3)
    );

    // Slice with rest pattern [first, .., last]
    let with_rest = Pattern::new(
        PatternKind::Slice {
            before: List::from(vec![Pattern::ident(test_ident("first"), false, span)]),
            rest: Maybe::Some(Heap::new(Pattern::new(PatternKind::Rest, span))),
            after: List::from(vec![Pattern::ident(test_ident("last"), false, span)]),
        },
        span,
    );
    assert!(matches!(with_rest.kind, PatternKind::Slice { rest, .. } if rest.is_some()));
}

#[test]
fn test_statement_construction() {
    let span = test_span();

    // Test expression statement
    let expr = Expr::literal(Literal::int(42, span));
    let expr_stmt = Stmt::new(
        StmtKind::Expr {
            expr,
            has_semi: true,
        },
        span,
    );
    assert!(matches!(expr_stmt.kind, StmtKind::Expr { .. }));

    // Test let statement
    let pattern = Pattern::ident(test_ident("x"), false, span);
    let value = Maybe::Some(Expr::literal(Literal::int(42, span)));
    let let_stmt = Stmt::new(
        StmtKind::Let {
            pattern,
            ty: Maybe::None,
            value,
        },
        span,
    );
    assert!(matches!(let_stmt.kind, StmtKind::Let { .. }));

    // Test let with type annotation
    let typed_pattern = Pattern::ident(test_ident("y"), false, span);
    let ty = Maybe::Some(Type::int(span));
    let typed_let = Stmt::new(
        StmtKind::Let {
            pattern: typed_pattern,
            ty,
            value: Maybe::None,
        },
        span,
    );
    assert!(matches!(typed_let.kind, StmtKind::Let { ty, .. } if ty.is_some()));
}

#[test]
fn test_block_construction() {
    let span = test_span();

    // Empty block
    let empty_block = Block {
        stmts: List::from(vec![]),
        expr: Maybe::None,
        span,
    };
    assert_eq!(empty_block.stmts.len(), 0);

    // Block with statements
    let stmts = vec![
        Stmt::new(
            StmtKind::Expr {
                expr: Expr::literal(Literal::int(1, span)),
                has_semi: true,
            },
            span,
        ),
        Stmt::new(
            StmtKind::Expr {
                expr: Expr::literal(Literal::int(2, span)),
                has_semi: true,
            },
            span,
        ),
    ];
    let block = Block {
        stmts: List::from(stmts.clone()),
        expr: Maybe::None,
        span,
    };
    assert_eq!(block.stmts.len(), 2);
}

#[test]
fn test_path_construction() {
    let span = test_span();

    // Single segment path
    let single = Path::single(test_ident("foo"));
    assert_eq!(single.segments.len(), 1);

    // Multi-segment path
    let segments = vec![
        PathSegment::Name(test_ident("std")),
        PathSegment::Name(test_ident("collections")),
        PathSegment::Name(test_ident("Vec")),
    ];
    let multi = Path::new(List::from(segments.clone()), span);
    assert_eq!(multi.segments.len(), 3);
    assert_eq!(multi.span, span);
}

#[test]
fn test_function_call_construction() {
    let span = test_span();

    // Simple function call: foo()
    let func = Heap::new(Expr::ident(test_ident("foo")));
    let call_expr = Expr::new(
        ExprKind::Call {
            func,
            args: List::from(vec![]),
            type_args: List::new(),
        },
        span,
    );
    assert!(matches!(call_expr.kind, ExprKind::Call { ref args, .. } if args.is_empty()));

    // Function call with arguments: foo(1, 2, 3)
    let func_with_args = Heap::new(Expr::ident(test_ident("foo")));
    let args = vec![
        Expr::literal(Literal::int(1, span)),
        Expr::literal(Literal::int(2, span)),
        Expr::literal(Literal::int(3, span)),
    ];
    let call_with_args = Expr::new(
        ExprKind::Call {
            func: func_with_args,
            type_args: List::new(),
            args: List::from(args.clone()),
        },
        span,
    );
    assert!(matches!(call_with_args.kind, ExprKind::Call { ref args, .. } if args.len() == 3));
}

#[test]
fn test_method_call_construction() {
    let span = test_span();

    // Simple method call: obj.method()
    let receiver = Heap::new(Expr::ident(test_ident("obj")));
    let method_call = Expr::new(
        ExprKind::MethodCall {
            receiver,
            method: test_ident("method"),
            type_args: List::new(),
            args: List::from(vec![]),
        },
        span,
    );
    assert!(matches!(method_call.kind, ExprKind::MethodCall { .. }));

    // Method call with arguments: obj.method(1, 2)
    let receiver_with_args = Heap::new(Expr::ident(test_ident("obj")));
    let args = vec![
        Expr::literal(Literal::int(1, span)),
        Expr::literal(Literal::int(2, span)),
    ];
    let method_with_args = Expr::new(
        ExprKind::MethodCall {
            receiver: receiver_with_args,
            method: test_ident("method"),
            type_args: List::new(),
            args: List::from(args),
        },
        span,
    );
    assert!(
        matches!(method_with_args.kind, ExprKind::MethodCall { ref args, .. } if args.len() == 2)
    );
}

#[test]
fn test_field_access_construction() {
    let span = test_span();

    // Regular field access: obj.field
    let obj = Heap::new(Expr::ident(test_ident("obj")));
    let field_access = Expr::new(
        ExprKind::Field {
            expr: obj,
            field: test_ident("field"),
        },
        span,
    );
    assert!(matches!(field_access.kind, ExprKind::Field { .. }));

    // Tuple field access: tuple.0
    let tuple = Heap::new(Expr::ident(test_ident("tuple")));
    let tuple_field = Expr::new(
        ExprKind::TupleIndex {
            expr: tuple,
            index: 0,
        },
        span,
    );
    assert!(matches!(
        tuple_field.kind,
        ExprKind::TupleIndex { index: 0, .. }
    ));

    // Optional chaining: obj?.field
    let optional_obj = Heap::new(Expr::ident(test_ident("obj")));
    let optional_chain = Expr::new(
        ExprKind::OptionalChain {
            expr: optional_obj,
            field: test_ident("field"),
        },
        span,
    );
    assert!(matches!(
        optional_chain.kind,
        ExprKind::OptionalChain { .. }
    ));
}

#[test]
fn test_complex_nested_expression() {
    let span = test_span();

    // Build a complex expression: (a + b) * (c - d)
    let a = Heap::new(Expr::ident(test_ident("a")));
    let b = Heap::new(Expr::ident(test_ident("b")));
    let add = Heap::new(Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: a,
            right: b,
        },
        span,
    ));

    let c = Heap::new(Expr::ident(test_ident("c")));
    let d = Heap::new(Expr::ident(test_ident("d")));
    let sub = Heap::new(Expr::new(
        ExprKind::Binary {
            op: BinOp::Sub,
            left: c,
            right: d,
        },
        span,
    ));

    let mul = Expr::new(
        ExprKind::Binary {
            op: BinOp::Mul,
            left: add,
            right: sub,
        },
        span,
    );

    // Verify the structure
    match mul.kind {
        ExprKind::Binary {
            op: BinOp::Mul,
            ref left,
            ref right,
        } => {
            assert!(matches!(left.kind, ExprKind::Binary { op: BinOp::Add, .. }));
            assert!(matches!(
                right.kind,
                ExprKind::Binary { op: BinOp::Sub, .. }
            ));
        }
        _ => panic!("Expected multiplication at the root"),
    }
}
