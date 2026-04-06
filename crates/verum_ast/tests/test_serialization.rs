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
//! Tests for serde serialization and deserialization of all AST nodes.
//!
//! This module ensures that all AST nodes can be correctly serialized
//! and deserialized, preserving all information through the round-trip.

use proptest::prelude::*;
use verum_ast::expr::*;
use verum_ast::literal::*;
use verum_ast::pattern::*;
use verum_ast::ty::PathSegment;
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

/// Helper macro to test round-trip serialization
macro_rules! test_round_trip {
    ($value:expr) => {{
        let original = $value;
        let json = serde_json::to_string(&original).expect("Failed to serialize");
        let deserialized = serde_json::from_str(&json).expect("Failed to deserialize");
        assert_eq!(
            original, deserialized,
            "Round-trip failed for {:?}",
            original
        );
        deserialized
    }};
}

#[test]
fn test_span_serialization() {
    let span = Span::new(10, 20, FileId::new(5));
    let result = test_round_trip!(span);
    assert_eq!(result.start, 10);
    assert_eq!(result.end, 20);
    assert_eq!(result.file_id, FileId::new(5));
}

#[test]
fn test_file_id_serialization() {
    let file_id = FileId::new(42);
    let result = test_round_trip!(file_id);
    assert_eq!(result, FileId::new(42));

    // Test dummy file ID
    let dummy = FileId::dummy();
    let dummy_result = test_round_trip!(dummy);
    assert_eq!(dummy_result, FileId::dummy());
}

#[test]
fn test_ident_serialization() {
    let ident = test_ident("test_identifier");
    let result = test_round_trip!(ident);
    assert_eq!(result.name.as_str(), "test_identifier");
    assert_eq!(result.span, test_span());
}

#[test]
fn test_path_serialization() {
    // Single segment path
    let single = Path::single(test_ident("foo"));
    let single_result = test_round_trip!(single);
    assert_eq!(single_result.segments.len(), 1);

    // Multi-segment path
    let multi = Path::new(
        List::from(vec![
            PathSegment::Name(test_ident("std")),
            PathSegment::Name(test_ident("io")),
            PathSegment::Name(test_ident("print")),
        ]),
        test_span(),
    );
    let multi_result = test_round_trip!(multi);
    assert_eq!(multi_result.segments.len(), 3);
}

#[test]
fn test_literal_serialization() {
    let span = test_span();

    // Integer literal
    let int_lit = Literal::int(42, span);
    let int_result = test_round_trip!(int_lit);
    assert!(matches!(int_result.kind, LiteralKind::Int(ref i) if i.value == 42));

    // Float literal
    let float_lit = Literal::float(3.14159, span);
    let float_result = test_round_trip!(float_lit);
    assert!(
        matches!(float_result.kind, LiteralKind::Float(ref f) if (f.value - 3.14159).abs() < 0.0001)
    );

    // String literal
    let str_lit = Literal::string("hello world".to_string().into(), span);
    let str_result = test_round_trip!(str_lit);
    assert!(
        matches!(str_result.kind, LiteralKind::Text(StringLit::Regular(ref s)) if s == "hello world")
    );

    // Boolean literals
    let bool_true = Literal::bool(true, span);
    let bool_true_result = test_round_trip!(bool_true);
    assert_eq!(bool_true_result.kind, LiteralKind::Bool(true));

    let bool_false = Literal::bool(false, span);
    let bool_false_result = test_round_trip!(bool_false);
    assert_eq!(bool_false_result.kind, LiteralKind::Bool(false));

    // Char literal
    let char_lit = Literal::char('🦀', span);
    let char_result = test_round_trip!(char_lit);
    assert_eq!(char_result.kind, LiteralKind::Char('🦀'));
}

#[test]
fn test_type_serialization() {
    let span = test_span();

    // Primitive types
    let unit_ty = Type::unit(span);
    test_round_trip!(unit_ty);

    let bool_ty = Type::bool(span);
    test_round_trip!(bool_ty);

    let int_ty = Type::int(span);
    test_round_trip!(int_ty);

    let float_ty = Type::float(span);
    test_round_trip!(float_ty);

    let text_ty = Type::text(span);
    test_round_trip!(text_ty);

    // Tuple type
    let tuple_ty = Type::new(
        TypeKind::Tuple(List::from(vec![Type::int(span), Type::text(span)])),
        span,
    );
    let tuple_result = test_round_trip!(tuple_ty);
    match tuple_result.kind {
        TypeKind::Tuple(elements) => {
            assert_eq!(elements.len(), 2);
            assert_eq!(elements.first().unwrap().kind, TypeKind::Int);
            assert_eq!(elements.get(1).unwrap().kind, TypeKind::Text);
        }
        _ => panic!("Expected tuple type"),
    }

    // Array type
    let array_ty = Type::new(
        TypeKind::Array {
            element: Heap::new(Type::int(span)),
            size: Maybe::Some(Heap::new(Expr::literal(Literal::int(10, span)))),
        },
        span,
    );
    test_round_trip!(array_ty);

    // Reference type
    let ref_ty = Type::new(
        TypeKind::Reference {
            mutable: true,
            inner: Heap::new(Type::int(span)),
        },
        span,
    );
    test_round_trip!(ref_ty);

    // Function type
    let fn_ty = Type::new(
        TypeKind::Function {
            params: List::from(vec![Type::int(span), Type::text(span)]),
            return_type: Heap::new(Type::bool(span)),
            calling_convention: Maybe::None,
            contexts: ContextList::empty(),
        },
        span,
    );
    test_round_trip!(fn_ty);
}

#[test]
fn test_pattern_serialization() {
    let span = test_span();

    // Wildcard pattern
    let wildcard = Pattern::wildcard(span);
    test_round_trip!(wildcard);

    // Identifier pattern
    let ident_pat = Pattern::ident(test_ident("x"), true, span);
    let ident_result = test_round_trip!(ident_pat);
    match ident_result.kind {
        PatternKind::Ident {
            mutable, ref name, ..
        } => {
            assert!(mutable);
            assert_eq!(name.name.as_str(), "x");
        }
        _ => panic!("Expected identifier pattern"),
    }

    // Literal pattern
    let lit_pat = Pattern::literal(Literal::int(42, span));
    test_round_trip!(lit_pat);

    // Tuple pattern
    let tuple_pat = Pattern::new(
        PatternKind::Tuple(List::from(vec![
            Pattern::wildcard(span),
            Pattern::ident(test_ident("y"), false, span),
        ])),
        span,
    );
    test_round_trip!(tuple_pat);

    // Slice pattern with rest
    let slice_pat = Pattern::new(
        PatternKind::Slice {
            before: List::from(vec![Pattern::ident(test_ident("first"), false, span)]),
            rest: Maybe::Some(Heap::new(Pattern::new(PatternKind::Rest, span))),
            after: List::from(vec![Pattern::ident(test_ident("last"), false, span)]),
        },
        span,
    );
    test_round_trip!(slice_pat);
}

#[test]
fn test_expression_serialization() {
    let span = test_span();

    // Literal expression
    let lit_expr = Expr::literal(Literal::string("test".to_string().into(), span));
    test_round_trip!(lit_expr);

    // Path expression
    let path_expr = Expr::path(Path::single(test_ident("variable")));
    test_round_trip!(path_expr);

    // Binary expression
    let binary_expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Heap::new(Expr::literal(Literal::int(1, span))),
            right: Heap::new(Expr::literal(Literal::int(2, span))),
        },
        span,
    );
    test_round_trip!(binary_expr);

    // Unary expression
    let unary_expr = Expr::new(
        ExprKind::Unary {
            op: UnOp::Not,
            expr: Heap::new(Expr::literal(Literal::bool(true, span))),
        },
        span,
    );
    test_round_trip!(unary_expr);

    // Function call
    let call_expr = Expr::new(
        ExprKind::Call {
            func: Heap::new(Expr::ident(test_ident("print"))),
            args: List::from(vec![Expr::literal(Literal::string(
                "hello".to_string().into(),
                span,
            ))]),
            type_args: List::new(),
        },
        span,
    );
    test_round_trip!(call_expr);

    // Method call
    let method_expr = Expr::new(
        ExprKind::MethodCall {
            receiver: Heap::new(Expr::ident(test_ident("obj"))),
            method: test_ident("method"),
            args: List::new(),
            type_args: List::new(),
        },
        span,
    );
    test_round_trip!(method_expr);

    // Field access
    let field_expr = Expr::new(
        ExprKind::Field {
            expr: Heap::new(Expr::ident(test_ident("obj"))),
            field: test_ident("field"),
        },
        span,
    );
    test_round_trip!(field_expr);

    // Array literal
    let array_expr = Expr::new(
        ExprKind::Array(ArrayExpr::List(List::from(vec![
            Expr::literal(Literal::int(1, span)),
            Expr::literal(Literal::int(2, span)),
            Expr::literal(Literal::int(3, span)),
        ]))),
        span,
    );
    test_round_trip!(array_expr);

    // Array repeat
    let array_repeat = Expr::new(
        ExprKind::Array(ArrayExpr::Repeat {
            value: Heap::new(Expr::literal(Literal::int(0, span))),
            count: Heap::new(Expr::literal(Literal::int(10, span))),
        }),
        span,
    );
    test_round_trip!(array_repeat);
}

#[test]
fn test_complex_expression_serialization() {
    let span = test_span();

    // If-else expression
    let if_expr = Expr::new(
        ExprKind::If {
            condition: Heap::new(IfCondition {
                conditions: smallvec::smallvec![ConditionKind::Expr(Expr::literal(Literal::bool(
                    true, span
                )))],
                span,
            }),
            then_branch: Block {
                stmts: List::from(vec![Stmt::new(
                    StmtKind::Expr {
                        expr: Expr::literal(Literal::int(1, span)),
                        has_semi: true,
                    },
                    span,
                )]),
                expr: Maybe::None,
                span,
            },
            else_branch: Maybe::Some(Heap::new(Expr::new(
                ExprKind::Block(Block {
                    stmts: List::from(vec![Stmt::new(
                        StmtKind::Expr {
                            expr: Expr::literal(Literal::int(2, span)),
                            has_semi: true,
                        },
                        span,
                    )]),
                    expr: Maybe::None,
                    span,
                }),
                span,
            ))),
        },
        span,
    );
    test_round_trip!(if_expr);

    // Match expression
    let match_expr = Expr::new(
        ExprKind::Match {
            expr: Heap::new(Expr::ident(test_ident("x"))),
            arms: List::from(vec![
                MatchArm {
                    attributes: verum_common::List::new(),
                    pattern: Pattern::literal(Literal::int(1, span)),
                    guard: Maybe::None,
                    body: Heap::new(Expr::literal(Literal::string("one".to_string().into(), span))),
                    with_clause: Maybe::None,
                    span,
                },
                MatchArm {
                    attributes: verum_common::List::new(),
                    pattern: Pattern::wildcard(span),
                    guard: Maybe::Some(Heap::new(Expr::literal(Literal::bool(true, span)))),
                    body: Heap::new(Expr::literal(Literal::string("other".to_string().into(), span))),
                    with_clause: Maybe::None,
                    span,
                },
            ]),
        },
        span,
    );
    test_round_trip!(match_expr);

    // Loop expression
    let loop_expr = Expr::new(
        ExprKind::Loop {
            label: Maybe::None,
            body: Block {
                stmts: List::from(vec![Stmt::new(
                    StmtKind::Expr {
                        expr: Expr::new(
                            ExprKind::Break {
                                label: Maybe::None,
                                value: Maybe::None,
                            },
                            span,
                        ),
                        has_semi: false,
                    },
                    span,
                )]),
                expr: Maybe::None,
                span,
            },
            invariants: List::new(),
        },
        span,
    );
    test_round_trip!(loop_expr);

    // While expression
    let while_expr = Expr::new(
        ExprKind::While {
            label: Maybe::None,
            condition: Heap::new(Expr::literal(Literal::bool(true, span))),
            body: Block {
                stmts: List::new(),
                expr: Maybe::None,
                span,
            },
            invariants: List::new(),
            decreases: List::new(),
        },
        span,
    );
    test_round_trip!(while_expr);

    // For loop
    let for_expr = Expr::new(
        ExprKind::For {
            label: Maybe::None,
            pattern: Pattern::ident(test_ident("i"), false, span),
            iter: Heap::new(Expr::ident(test_ident("range"))),
            body: Block {
                stmts: List::new(),
                expr: Maybe::None,
                span,
            },
            invariants: List::new(),
            decreases: List::new(),
        },
        span,
    );
    test_round_trip!(for_expr);
}

#[test]
fn test_statement_serialization() {
    let span = test_span();

    // Expression statement
    let expr_stmt = Stmt::new(
        StmtKind::Expr {
            expr: Expr::literal(Literal::int(42, span)),
            has_semi: true,
        },
        span,
    );
    test_round_trip!(expr_stmt);

    // Let statement
    let let_stmt = Stmt::new(
        StmtKind::Let {
            pattern: Pattern::ident(test_ident("x"), false, span),
            ty: Maybe::Some(Type::int(span)),
            value: Maybe::Some(Expr::literal(Literal::int(42, span))),
        },
        span,
    );
    test_round_trip!(let_stmt);

    // Assignment (now as expression statement)
    let assign_stmt = Stmt::new(
        StmtKind::Expr {
            expr: Expr::new(
                ExprKind::Binary {
                    op: BinOp::Assign,
                    left: Heap::new(Expr::ident(test_ident("x"))),
                    right: Heap::new(Expr::literal(Literal::int(100, span))),
                },
                span,
            ),
            has_semi: true,
        },
        span,
    );
    test_round_trip!(assign_stmt);

    // Return statement (now as expression statement)
    let return_stmt = Stmt::new(
        StmtKind::Expr {
            expr: Expr::new(
                ExprKind::Return(Maybe::Some(Heap::new(Expr::ident(test_ident("result"))))),
                span,
            ),
            has_semi: false,
        },
        span,
    );
    test_round_trip!(return_stmt);

    // Break expression (as statement)
    let break_stmt = Stmt::new(
        StmtKind::Expr {
            expr: Expr::new(
                ExprKind::Break {
                    label: Maybe::None,
                    value: Maybe::Some(Heap::new(Expr::literal(Literal::int(0, span)))),
                },
                span,
            ),
            has_semi: false,
        },
        span,
    );
    test_round_trip!(break_stmt);

    // Continue expression (as statement)
    let continue_stmt = Stmt::new(
        StmtKind::Expr {
            expr: Expr::new(ExprKind::Continue { label: Maybe::None }, span),
            has_semi: false,
        },
        span,
    );
    test_round_trip!(continue_stmt);
}

#[test]
fn test_block_serialization() {
    let span = test_span();

    let block = Block {
        stmts: List::from(vec![
            Stmt::new(
                StmtKind::Let {
                    pattern: Pattern::ident(test_ident("x"), false, span),
                    ty: Maybe::None,
                    value: Maybe::Some(Expr::literal(Literal::int(1, span))),
                },
                span,
            ),
            Stmt::new(
                StmtKind::Let {
                    pattern: Pattern::ident(test_ident("y"), false, span),
                    ty: Maybe::None,
                    value: Maybe::Some(Expr::literal(Literal::int(2, span))),
                },
                span,
            ),
            Stmt::new(
                StmtKind::Expr {
                    expr: Expr::new(
                        ExprKind::Binary {
                            op: BinOp::Add,
                            left: Heap::new(Expr::ident(test_ident("x"))),
                            right: Heap::new(Expr::ident(test_ident("y"))),
                        },
                        span,
                    ),
                    has_semi: false,
                },
                span,
            ),
        ]),
        expr: Maybe::None,
        span,
    };

    test_round_trip!(block);
}

#[test]
fn test_module_serialization() {
    let file_id = FileId::new(0);
    let span = test_span();

    let module = Module::new(
        List::from(vec![Item::new(
            ItemKind::Mount(MountDecl {
                visibility: Visibility::Private,
                tree: MountTree { alias: Maybe::None,
                    kind: MountTreeKind::Path(Path::new(
                        List::from(vec![
                            PathSegment::Name(test_ident("std")),
                            PathSegment::Name(test_ident("io")),
                        ]),
                        span,
                    )),
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

    test_round_trip!(module);
}

#[test]
fn test_compilation_unit_serialization() {
    let module1 = Module::empty(FileId::new(0));
    let module2 = Module::empty(FileId::new(1));

    let unit = CompilationUnit::new(List::from(vec![module1, module2]));
    let result = test_round_trip!(unit);

    assert_eq!(result.modules.len(), 2);
    assert_eq!(result.modules.first().unwrap().file_id, FileId::new(0));
    assert_eq!(result.modules.get(1).unwrap().file_id, FileId::new(1));
}

#[test]
fn test_closure_serialization() {
    let span = test_span();

    let closure = Expr::new(
        ExprKind::Closure {
            async_: false,
            move_: false,
            params: List::from(vec![
                ClosureParam {
                    pattern: Pattern::ident(test_ident("x"), false, span),
                    ty: Maybe::Some(Type::int(span)),
                    span,
                },
                ClosureParam {
                    pattern: Pattern::ident(test_ident("y"), false, span),
                    ty: Maybe::Some(Type::int(span)),
                    span,
                },
            ]),
            contexts: List::new(),
            return_type: Maybe::Some(Type::int(span)),
            body: Heap::new(Expr::new(
                ExprKind::Binary {
                    op: BinOp::Add,
                    left: Heap::new(Expr::ident(test_ident("x"))),
                    right: Heap::new(Expr::ident(test_ident("y"))),
                },
                span,
            )),
        },
        span,
    );

    test_round_trip!(closure);
}

#[test]
fn test_stream_comprehension_serialization() {
    let span = test_span();

    let comprehension = Expr::new(
        ExprKind::StreamComprehension {
            expr: Heap::new(Expr::new(
                ExprKind::Binary {
                    op: BinOp::Mul,
                    left: Heap::new(Expr::ident(test_ident("x"))),
                    right: Heap::new(Expr::literal(Literal::int(2, span))),
                },
                span,
            )),
            clauses: List::from(vec![
                ComprehensionClause {
                    kind: ComprehensionClauseKind::For {
                        pattern: Pattern::ident(test_ident("x"), false, span),
                        iter: Expr::ident(test_ident("numbers")),
                    },
                    span,
                },
                ComprehensionClause {
                    kind: ComprehensionClauseKind::If(Expr::new(
                        ExprKind::Binary {
                            op: BinOp::Gt,
                            left: Heap::new(Expr::ident(test_ident("x"))),
                            right: Heap::new(Expr::literal(Literal::int(0, span))),
                        },
                        span,
                    )),
                    span,
                },
            ]),
        },
        span,
    );

    test_round_trip!(comprehension);
}

#[test]
fn test_refinement_type_serialization() {
    let span = test_span();

    // Create a refined type: Int{> 0}
    let refinement = Type::new(
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

    let result = test_round_trip!(refinement);

    match result.kind {
        TypeKind::Refined {
            ref base,
            ref predicate,
        } => {
            assert_eq!(base.kind, TypeKind::Int);
            // Verify the predicate expression
            match &predicate.expr.kind {
                ExprKind::Binary { op: BinOp::Gt, .. } => {}
                _ => panic!("Expected greater-than comparison in refinement"),
            }
        }
        _ => panic!("Expected refined type"),
    }
}

// Property-based tests for serialization
proptest! {
    #[test]
    fn prop_span_serialization(start in 0u32..10000, end in 0u32..10000, file_id in 0u32..100) {
        let span = Span::new(start, end, FileId::new(file_id));
        let json = serde_json::to_string(&span).unwrap();
        let deserialized: Span = serde_json::from_str(&json).unwrap();
        assert_eq!(span, deserialized);
    }

    #[test]
    fn prop_literal_int_serialization(value in any::<i64>()) {
        let lit = Literal::int(value as i128, test_span());
        let json = serde_json::to_string(&lit).unwrap();
        let deserialized: Literal = serde_json::from_str(&json).unwrap();
        assert_eq!(lit, deserialized);
    }

    #[test]
    fn prop_literal_string_serialization(value in "\\PC*") {
        let lit = Literal::string(value.into(), test_span());
        let json = serde_json::to_string(&lit).unwrap();
        let deserialized: Literal = serde_json::from_str(&json).unwrap();
        assert_eq!(lit, deserialized);
    }

    #[test]
    fn prop_ident_serialization(name in "[a-zA-Z_][a-zA-Z0-9_]*") {
        let ident = Ident::new(name.clone(), test_span());
        let json = serde_json::to_string(&ident).unwrap();
        let deserialized: Ident = serde_json::from_str(&json).unwrap();
        assert_eq!(ident, deserialized);
    }
}

#[test]
fn test_json_format_stability() {
    // Test that the JSON format is stable and human-readable
    let span = test_span();
    let expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Heap::new(Expr::literal(Literal::int(1, span))),
            right: Heap::new(Expr::literal(Literal::int(2, span))),
        },
        span,
    );

    let json = serde_json::to_string_pretty(&expr).unwrap();

    // Verify the JSON contains expected fields
    assert!(json.contains("\"kind\""));
    assert!(json.contains("\"Binary\""));
    assert!(json.contains("\"Add\""));
    assert!(json.contains("\"left\""));
    assert!(json.contains("\"right\""));
    assert!(json.contains("\"span\""));

    // Verify we can parse it back
    let parsed: Expr = serde_json::from_str(&json).unwrap();
    assert_eq!(expr, parsed);
}

#[test]
fn test_large_ast_serialization() {
    let span = test_span();

    // Create a large nested expression tree
    // Using 20 levels to stay within serde_json's recursion limit (default 128)
    // Each Binary expression adds multiple levels due to nested structures
    let mut expr = Expr::literal(Literal::int(0, span));

    for i in 1..20 {
        expr = Expr::new(
            ExprKind::Binary {
                op: if i % 2 == 0 { BinOp::Add } else { BinOp::Mul },
                left: Heap::new(expr),
                right: Heap::new(Expr::literal(Literal::int(i, span))),
            },
            span,
        );
    }

    // Should be able to serialize and deserialize large ASTs
    test_round_trip!(expr);
}

#[test]
fn test_unicode_in_serialization() {
    let span = test_span();

    // Test unicode in identifiers and strings
    let unicode_ident = Ident::new("变量_名称_🦀".to_string(), span);
    test_round_trip!(unicode_ident);

    let unicode_string = Literal::string("Hello, 世界! 🌍".to_string().into(), span);
    test_round_trip!(unicode_string);

    let unicode_char = Literal::char('🦀', span);
    test_round_trip!(unicode_char);
}
