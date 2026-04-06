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
//! Tests for PartialEq implementations of all AST nodes.
//!
//! This module ensures that equality comparisons work correctly
//! for all AST node types, considering all fields.

use verum_ast::expr::*;
use verum_ast::pattern::*;
use verum_ast::*;
use verum_common::List;
use verum_common::{Heap, Maybe};

/// Helper function to create a test span
fn test_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

/// Helper function to create a different test span
fn other_span() -> Span {
    Span::new(10, 20, FileId::new(1))
}

/// Helper function to create a test identifier
fn test_ident(name: &str) -> Ident {
    Ident::new(name.to_string(), test_span())
}

#[test]
fn test_span_equality() {
    let span1 = Span::new(0, 10, FileId::new(0));
    let span2 = Span::new(0, 10, FileId::new(0));
    let span3 = Span::new(0, 10, FileId::new(1)); // Different file
    let span4 = Span::new(5, 10, FileId::new(0)); // Different start
    let span5 = Span::new(0, 15, FileId::new(0)); // Different end

    // Equal spans
    assert_eq!(span1, span2);
    assert_eq!(span1, span1); // Reflexivity

    // Different spans
    assert_ne!(span1, span3);
    assert_ne!(span1, span4);
    assert_ne!(span1, span5);

    // Dummy span equality
    assert_eq!(Span::dummy(), Span::dummy());
}

#[test]
fn test_file_id_equality() {
    let id1 = FileId::new(0);
    let id2 = FileId::new(0);
    let id3 = FileId::new(1);

    assert_eq!(id1, id2);
    assert_ne!(id1, id3);
    assert_eq!(FileId::dummy(), FileId::dummy());
    assert_ne!(id1, FileId::dummy());
}

#[test]
fn test_ident_equality() {
    let ident1 = test_ident("foo");
    let ident2 = test_ident("foo");
    let ident3 = test_ident("bar");
    let ident4 = Ident::new("foo".to_string(), other_span());

    // Same name and span
    assert_eq!(ident1, ident2);

    // Different name
    assert_ne!(ident1, ident3);

    // Same name, different span (spans are compared!)
    assert_ne!(ident1, ident4);
}

#[test]
fn test_path_equality() {
    let path1 = Path::single(test_ident("foo"));
    let path2 = Path::single(test_ident("foo"));
    let path3 = Path::single(test_ident("bar"));

    assert_eq!(path1, path2);
    assert_ne!(path1, path3);

    // Multi-segment paths
    let multi1 = Path::new(
        List::from(vec![
            PathSegment::Name(test_ident("std")),
            PathSegment::Name(test_ident("io")),
        ]),
        test_span(),
    );
    let multi2 = Path::new(
        List::from(vec![
            PathSegment::Name(test_ident("std")),
            PathSegment::Name(test_ident("io")),
        ]),
        test_span(),
    );
    let multi3 = Path::new(
        List::from(vec![
            PathSegment::Name(test_ident("std")),
            PathSegment::Name(test_ident("fs")),
        ]),
        test_span(),
    );

    assert_eq!(multi1, multi2);
    assert_ne!(multi1, multi3);

    // Different lengths
    let short = Path::single(test_ident("std"));
    assert_ne!(short, multi1);
}

#[test]
fn test_literal_equality() {
    let span = test_span();

    // Integer literals
    assert_eq!(Literal::int(42, span), Literal::int(42, span));
    assert_ne!(Literal::int(42, span), Literal::int(43, span));

    // Float literals
    assert_eq!(Literal::float(3.14, span), Literal::float(3.14, span));
    assert_ne!(Literal::float(3.14, span), Literal::float(2.71, span));

    // String literals
    assert_eq!(
        Literal::string("hello".to_string().into(), span),
        Literal::string("hello".to_string().into(), span)
    );
    assert_ne!(
        Literal::string("hello".to_string().into(), span),
        Literal::string("world".to_string().into(), span)
    );

    // Boolean literals
    assert_eq!(Literal::bool(true, span), Literal::bool(true, span));
    assert_eq!(Literal::bool(false, span), Literal::bool(false, span));
    assert_ne!(Literal::bool(true, span), Literal::bool(false, span));

    // Char literals
    assert_eq!(Literal::char('a', span), Literal::char('a', span));
    assert_ne!(Literal::char('a', span), Literal::char('b', span));

    // Unit literals
    // Unit type is not a literal anymore, removed test

    // Different types
    assert_ne!(Literal::int(42, span), Literal::float(42.0, span));
    assert_ne!(
        Literal::string("42".to_string().into(), span),
        Literal::int(42, span)
    );
}

#[test]
fn test_type_equality() {
    let span = test_span();

    // Primitive types
    assert_eq!(Type::int(span), Type::int(span));
    assert_eq!(Type::bool(span), Type::bool(span));
    assert_ne!(Type::int(span), Type::bool(span));
    assert_ne!(Type::float(span), Type::text(span));

    // Tuple types
    let tuple1 = Type::new(
        TypeKind::Tuple(List::from(vec![Type::int(span), Type::bool(span)])),
        span,
    );
    let tuple2 = Type::new(
        TypeKind::Tuple(List::from(vec![Type::int(span), Type::bool(span)])),
        span,
    );
    let tuple3 = Type::new(
        TypeKind::Tuple(List::from(vec![Type::bool(span), Type::int(span)])), // Swapped
        span,
    );

    assert_eq!(tuple1, tuple2);
    assert_ne!(tuple1, tuple3);

    // Array types
    let array1 = Type::new(
        TypeKind::Array {
            element: Heap::new(Type::int(span)),
            size: Maybe::Some(Heap::new(Expr::literal(Literal::int(10, span)))),
        },
        span,
    );
    let array2 = Type::new(
        TypeKind::Array {
            element: Heap::new(Type::int(span)),
            size: Maybe::Some(Heap::new(Expr::literal(Literal::int(10, span)))),
        },
        span,
    );
    let array3 = Type::new(
        TypeKind::Array {
            element: Heap::new(Type::int(span)),
            size: Maybe::Some(Heap::new(Expr::literal(Literal::int(20, span)))), // Different size
        },
        span,
    );

    assert_eq!(array1, array2);
    assert_ne!(array1, array3);
}

#[test]
fn test_pattern_equality() {
    let span = test_span();

    // Wildcard patterns
    assert_eq!(Pattern::wildcard(span), Pattern::wildcard(span));

    // Identifier patterns
    let ident1 = Pattern::ident(test_ident("x"), false, span);
    let ident2 = Pattern::ident(test_ident("x"), false, span);
    let ident3 = Pattern::ident(test_ident("y"), false, span); // Different name
    let ident4 = Pattern::ident(test_ident("x"), true, span); // Different mutability

    assert_eq!(ident1, ident2);
    assert_ne!(ident1, ident3);
    assert_ne!(ident1, ident4);

    // Literal patterns
    let lit1 = Pattern::literal(Literal::int(42, span));
    let lit2 = Pattern::literal(Literal::int(42, span));
    let lit3 = Pattern::literal(Literal::int(43, span));

    assert_eq!(lit1, lit2);
    assert_ne!(lit1, lit3);

    // Tuple patterns
    let tuple1 = Pattern::new(
        PatternKind::Tuple(List::from(vec![
            Pattern::wildcard(span),
            Pattern::ident(test_ident("x"), false, span),
        ])),
        span,
    );
    let tuple2 = Pattern::new(
        PatternKind::Tuple(List::from(vec![
            Pattern::wildcard(span),
            Pattern::ident(test_ident("x"), false, span),
        ])),
        span,
    );
    let tuple3 = Pattern::new(
        PatternKind::Tuple(List::from(vec![
            Pattern::wildcard(span),
            Pattern::ident(test_ident("y"), false, span), // Different ident
        ])),
        span,
    );

    assert_eq!(tuple1, tuple2);
    assert_ne!(tuple1, tuple3);
}

#[test]
fn test_expression_equality() {
    let span = test_span();

    // Literal expressions
    let lit1 = Expr::literal(Literal::int(42, span));
    let lit2 = Expr::literal(Literal::int(42, span));
    let lit3 = Expr::literal(Literal::int(43, span));

    assert_eq!(lit1, lit2);
    assert_ne!(lit1, lit3);

    // Path expressions
    let path1 = Expr::ident(test_ident("x"));
    let path2 = Expr::ident(test_ident("x"));
    let path3 = Expr::ident(test_ident("y"));

    assert_eq!(path1, path2);
    assert_ne!(path1, path3);

    // Binary expressions
    let binary1 = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Heap::new(Expr::literal(Literal::int(1, span))),
            right: Heap::new(Expr::literal(Literal::int(2, span))),
        },
        span,
    );
    let binary2 = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Heap::new(Expr::literal(Literal::int(1, span))),
            right: Heap::new(Expr::literal(Literal::int(2, span))),
        },
        span,
    );
    let binary3 = Expr::new(
        ExprKind::Binary {
            op: BinOp::Sub, // Different operator
            left: Heap::new(Expr::literal(Literal::int(1, span))),
            right: Heap::new(Expr::literal(Literal::int(2, span))),
        },
        span,
    );
    let binary4 = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Heap::new(Expr::literal(Literal::int(2, span))), // Swapped operands
            right: Heap::new(Expr::literal(Literal::int(1, span))),
        },
        span,
    );

    assert_eq!(binary1, binary2);
    assert_ne!(binary1, binary3);
    assert_ne!(binary1, binary4); // Addition is not commutative in AST
}

#[test]
fn test_statement_equality() {
    let span = test_span();

    // Expression statements
    let expr1 = Stmt::new(
        StmtKind::Expr {
            expr: Expr::literal(Literal::int(42, span)),
            has_semi: true,
        },
        span,
    );
    let expr2 = Stmt::new(
        StmtKind::Expr {
            expr: Expr::literal(Literal::int(42, span)),
            has_semi: true,
        },
        span,
    );
    let expr3 = Stmt::new(
        StmtKind::Expr {
            expr: Expr::literal(Literal::int(43, span)),
            has_semi: true,
        },
        span,
    );

    assert_eq!(expr1, expr2);
    assert_ne!(expr1, expr3);

    // Let statements
    let let1 = Stmt::new(
        StmtKind::Let {
            pattern: Pattern::ident(test_ident("x"), false, span),
            ty: Maybe::Some(Type::int(span)),
            value: Maybe::Some(Expr::literal(Literal::int(42, span))),
        },
        span,
    );
    let let2 = Stmt::new(
        StmtKind::Let {
            pattern: Pattern::ident(test_ident("x"), false, span),
            ty: Maybe::Some(Type::int(span)),
            value: Maybe::Some(Expr::literal(Literal::int(42, span))),
        },
        span,
    );
    let let3 = Stmt::new(
        StmtKind::Let {
            pattern: Pattern::ident(test_ident("y"), false, span), // Different name
            ty: Maybe::Some(Type::int(span)),
            value: Maybe::Some(Expr::literal(Literal::int(42, span))),
        },
        span,
    );

    assert_eq!(let1, let2);
    assert_ne!(let1, let3);
}

#[test]
fn test_block_equality() {
    let span = test_span();

    let block1 = Block {
        stmts: List::from(vec![
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
        ]),
        expr: Maybe::None,
        span,
    };

    let block2 = Block {
        stmts: List::from(vec![
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
        ]),
        expr: Maybe::None,
        span,
    };

    let block3 = Block {
        stmts: List::from(vec![
            Stmt::new(
                StmtKind::Expr {
                    expr: Expr::literal(Literal::int(2, span)),
                    has_semi: true,
                }, // Swapped
                span,
            ),
            Stmt::new(
                StmtKind::Expr {
                    expr: Expr::literal(Literal::int(1, span)),
                    has_semi: true,
                },
                span,
            ),
        ]),
        expr: Maybe::None,
        span,
    };

    assert_eq!(block1, block2);
    assert_ne!(block1, block3); // Order matters
}

#[test]
fn test_module_equality() {
    let file_id = FileId::new(0);
    let span = test_span();

    let module1 = Module::new(
        List::from(vec![Item::new(
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
        )]),
        file_id,
        span,
    );

    let module2 = Module::new(
        List::from(vec![Item::new(
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
        )]),
        file_id,
        span,
    );

    let module3 = Module::new(
        List::from(vec![Item::new(
            ItemKind::Mount(MountDecl {
                visibility: Visibility::Private,
                tree: MountTree { alias: Maybe::None,
                    kind: MountTreeKind::Path(Path::single(test_ident("io"))), // Different mount
                    span,
                },
                alias: Maybe::None,
                span,
            }),
            span,
        )]),
        file_id,
        span,
    );

    assert_eq!(module1, module2);
    assert_ne!(module1, module3);
}

#[test]
fn test_deep_nested_equality() {
    let span = test_span();

    // Create deeply nested expressions
    let deep1 = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Heap::new(Expr::new(
                ExprKind::Binary {
                    op: BinOp::Mul,
                    left: Heap::new(Expr::literal(Literal::int(1, span))),
                    right: Heap::new(Expr::literal(Literal::int(2, span))),
                },
                span,
            )),
            right: Heap::new(Expr::new(
                ExprKind::Binary {
                    op: BinOp::Sub,
                    left: Heap::new(Expr::literal(Literal::int(3, span))),
                    right: Heap::new(Expr::literal(Literal::int(4, span))),
                },
                span,
            )),
        },
        span,
    );

    let deep2 = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Heap::new(Expr::new(
                ExprKind::Binary {
                    op: BinOp::Mul,
                    left: Heap::new(Expr::literal(Literal::int(1, span))),
                    right: Heap::new(Expr::literal(Literal::int(2, span))),
                },
                span,
            )),
            right: Heap::new(Expr::new(
                ExprKind::Binary {
                    op: BinOp::Sub,
                    left: Heap::new(Expr::literal(Literal::int(3, span))),
                    right: Heap::new(Expr::literal(Literal::int(4, span))),
                },
                span,
            )),
        },
        span,
    );

    // Same structure, different value deep inside
    let deep3 = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Heap::new(Expr::new(
                ExprKind::Binary {
                    op: BinOp::Mul,
                    left: Heap::new(Expr::literal(Literal::int(1, span))),
                    right: Heap::new(Expr::literal(Literal::int(2, span))),
                },
                span,
            )),
            right: Heap::new(Expr::new(
                ExprKind::Binary {
                    op: BinOp::Sub,
                    left: Heap::new(Expr::literal(Literal::int(3, span))),
                    right: Heap::new(Expr::literal(Literal::int(5, span))), // Changed from 4 to 5
                },
                span,
            )),
        },
        span,
    );

    assert_eq!(deep1, deep2);
    assert_ne!(deep1, deep3);
}

#[test]
fn test_match_arm_equality() {
    let span = test_span();

    let arm1 = MatchArm {
        attributes: verum_common::List::new(),
        pattern: Pattern::literal(Literal::int(1, span)),
        guard: Maybe::None,
        body: Heap::new(Expr::literal(Literal::string("one".to_string().into(), span))),
        with_clause: Maybe::None,
        span,
    };

    let arm2 = MatchArm {
        attributes: verum_common::List::new(),
        pattern: Pattern::literal(Literal::int(1, span)),
        guard: Maybe::None,
        body: Heap::new(Expr::literal(Literal::string("one".to_string().into(), span))),
        with_clause: Maybe::None,
        span,
    };

    let arm3 = MatchArm {
        attributes: verum_common::List::new(),
        pattern: Pattern::literal(Literal::int(2, span)), // Different pattern
        guard: Maybe::None,
        body: Heap::new(Expr::literal(Literal::string("one".to_string().into(), span))),
        with_clause: Maybe::None,
        span,
    };

    let arm4 = MatchArm {
        attributes: verum_common::List::new(),
        pattern: Pattern::literal(Literal::int(1, span)),
        guard: Maybe::Some(Heap::new(Expr::literal(Literal::bool(true, span)))), // Added guard
        body: Heap::new(Expr::literal(Literal::string("one".to_string().into(), span))),
        with_clause: Maybe::None,
        span,
    };

    assert_eq!(arm1, arm2);
    assert_ne!(arm1, arm3);
    assert_ne!(arm1, arm4);
}

#[test]
fn test_closure_equality() {
    let span = test_span();

    let closure1 = Expr::new(
        ExprKind::Closure {
            async_: false,
            move_: false,
            params: List::from(vec![ClosureParam {
                pattern: Pattern::ident(test_ident("x"), false, span),
                ty: Maybe::Some(Type::int(span)),
                span,
            }]),
            contexts: List::new(),
            return_type: Maybe::Some(Type::int(span)),
            body: Heap::new(Expr::ident(test_ident("x"))),
        },
        span,
    );

    let closure2 = Expr::new(
        ExprKind::Closure {
            async_: false,
            move_: false,
            params: List::from(vec![ClosureParam {
                pattern: Pattern::ident(test_ident("x"), false, span),
                ty: Maybe::Some(Type::int(span)),
                span,
            }]),
            contexts: List::new(),
            return_type: Maybe::Some(Type::int(span)),
            body: Heap::new(Expr::ident(test_ident("x"))),
        },
        span,
    );

    let closure3 = Expr::new(
        ExprKind::Closure {
            async_: false,
            move_: false,
            params: List::from(vec![ClosureParam {
                pattern: Pattern::ident(test_ident("y"), false, span), // Different param name
                ty: Maybe::Some(Type::int(span)),
                span,
            }]),
            contexts: List::new(),
            return_type: Maybe::Some(Type::int(span)),
            body: Heap::new(Expr::ident(test_ident("y"))),
        },
        span,
    );

    assert_eq!(closure1, closure2);
    assert_ne!(closure1, closure3);
}

#[test]
fn test_array_expr_equality() {
    let span = test_span();

    // Array with elements
    let array1 = Expr::new(
        ExprKind::Array(ArrayExpr::List(List::from(vec![
            Expr::literal(Literal::int(1, span)),
            Expr::literal(Literal::int(2, span)),
        ]))),
        span,
    );

    let array2 = Expr::new(
        ExprKind::Array(ArrayExpr::List(List::from(vec![
            Expr::literal(Literal::int(1, span)),
            Expr::literal(Literal::int(2, span)),
        ]))),
        span,
    );

    let array3 = Expr::new(
        ExprKind::Array(ArrayExpr::List(List::from(vec![
            Expr::literal(Literal::int(2, span)), // Swapped
            Expr::literal(Literal::int(1, span)),
        ]))),
        span,
    );

    assert_eq!(array1, array2);
    assert_ne!(array1, array3);

    // Array repeat
    let repeat1 = Expr::new(
        ExprKind::Array(ArrayExpr::Repeat {
            value: Heap::new(Expr::literal(Literal::int(0, span))),
            count: Heap::new(Expr::literal(Literal::int(10, span))),
        }),
        span,
    );

    let repeat2 = Expr::new(
        ExprKind::Array(ArrayExpr::Repeat {
            value: Heap::new(Expr::literal(Literal::int(0, span))),
            count: Heap::new(Expr::literal(Literal::int(10, span))),
        }),
        span,
    );

    let repeat3 = Expr::new(
        ExprKind::Array(ArrayExpr::Repeat {
            value: Heap::new(Expr::literal(Literal::int(0, span))),
            count: Heap::new(Expr::literal(Literal::int(20, span))), // Different count
        }),
        span,
    );

    assert_eq!(repeat1, repeat2);
    assert_ne!(repeat1, repeat3);
}

#[test]
fn test_stream_comprehension_equality() {
    let span = test_span();

    let comp1 = Expr::new(
        ExprKind::StreamComprehension {
            expr: Heap::new(Expr::ident(test_ident("x"))),
            clauses: List::from(vec![ComprehensionClause {
                kind: ComprehensionClauseKind::For {
                    pattern: Pattern::ident(test_ident("x"), false, span),
                    iter: Expr::ident(test_ident("list")),
                },
                span,
            }]),
        },
        span,
    );

    let comp2 = Expr::new(
        ExprKind::StreamComprehension {
            expr: Heap::new(Expr::ident(test_ident("x"))),
            clauses: List::from(vec![ComprehensionClause {
                kind: ComprehensionClauseKind::For {
                    pattern: Pattern::ident(test_ident("x"), false, span),
                    iter: Expr::ident(test_ident("list")),
                },
                span,
            }]),
        },
        span,
    );

    let comp3 = Expr::new(
        ExprKind::StreamComprehension {
            expr: Heap::new(Expr::ident(test_ident("y"))), // Different element
            clauses: List::from(vec![ComprehensionClause {
                kind: ComprehensionClauseKind::For {
                    pattern: Pattern::ident(test_ident("y"), false, span),
                    iter: Expr::ident(test_ident("list")),
                },
                span,
            }]),
        },
        span,
    );

    assert_eq!(comp1, comp2);
    assert_ne!(comp1, comp3);
}

#[test]
fn test_refinement_type_equality() {
    let span = test_span();

    let refined1 = Type::new(
        TypeKind::Refined {
            base: Heap::new(Type::int(span)),
            predicate: Heap::new(RefinementPredicate {
                expr: Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Gt,
                        left: Heap::new(Expr::ident(test_ident("it"))),
                        right: Heap::new(Expr::literal(Literal::int(0, span))),
                    },
                    span,
                ),
                binding: Maybe::None,
                span,
            }),
        },
        span,
    );

    let refined2 = Type::new(
        TypeKind::Refined {
            base: Heap::new(Type::int(span)),
            predicate: Heap::new(RefinementPredicate {
                expr: Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Gt,
                        left: Heap::new(Expr::ident(test_ident("it"))),
                        right: Heap::new(Expr::literal(Literal::int(0, span))),
                    },
                    span,
                ),
                binding: Maybe::None,
                span,
            }),
        },
        span,
    );

    let refined3 = Type::new(
        TypeKind::Refined {
            base: Heap::new(Type::int(span)),
            predicate: Heap::new(RefinementPredicate {
                expr: Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Ge, // Different operator
                        left: Heap::new(Expr::ident(test_ident("it"))),
                        right: Heap::new(Expr::literal(Literal::int(0, span))),
                    },
                    span,
                ),
                binding: Maybe::None,
                span,
            }),
        },
        span,
    );

    assert_eq!(refined1, refined2);
    assert_ne!(refined1, refined3);
}
