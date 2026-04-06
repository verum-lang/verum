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
//! Tests for complex expression trees.
//!
//! This module tests various complex expression structures,
//! including deeply nested expressions, control flow, and special forms.

use proptest::prelude::*;
use smallvec::smallvec;
use verum_ast::expr::*;
use verum_ast::pattern::*;
use verum_ast::*;
use verum_common::Heap;
use verum_common::List;
use verum_common::Maybe;

/// Helper function to create a test span
fn test_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

/// Helper function to create a test identifier
fn test_ident(name: &str) -> Ident {
    Ident::new(name.to_string(), test_span())
}

#[test]
fn test_arithmetic_expression_tree() {
    let span = test_span();

    // Build expression: (a + b) * (c - d) / e
    let add = Heap::new(Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Heap::new(Expr::ident(test_ident("a"))),
            right: Heap::new(Expr::ident(test_ident("b"))),
        },
        span,
    ));

    let sub = Heap::new(Expr::new(
        ExprKind::Binary {
            op: BinOp::Sub,
            left: Heap::new(Expr::ident(test_ident("c"))),
            right: Heap::new(Expr::ident(test_ident("d"))),
        },
        span,
    ));

    let mul = Heap::new(Expr::new(
        ExprKind::Binary {
            op: BinOp::Mul,
            left: add,
            right: sub,
        },
        span,
    ));

    let div = Expr::new(
        ExprKind::Binary {
            op: BinOp::Div,
            left: mul,
            right: Heap::new(Expr::ident(test_ident("e"))),
        },
        span,
    );

    // Verify the structure
    match &div.kind {
        ExprKind::Binary {
            op: BinOp::Div,
            left,
            right,
        } => {
            // Check left is multiplication
            match &left.kind {
                ExprKind::Binary { op: BinOp::Mul, .. } => {}
                _ => panic!("Expected multiplication on left"),
            }
            // Check right is identifier
            match &right.kind {
                ExprKind::Path(_) => {}
                _ => panic!("Expected identifier on right"),
            }
        }
        _ => panic!("Expected division at root"),
    }
}

#[test]
fn test_logical_expression_tree() {
    let span = test_span();

    // Build: (a && b) || (c && !d)
    let and1 = Heap::new(Expr::new(
        ExprKind::Binary {
            op: BinOp::And,
            left: Heap::new(Expr::ident(test_ident("a"))),
            right: Heap::new(Expr::ident(test_ident("b"))),
        },
        span,
    ));

    let not_d = Heap::new(Expr::new(
        ExprKind::Unary {
            op: UnOp::Not,
            expr: Heap::new(Expr::ident(test_ident("d"))),
        },
        span,
    ));

    let and2 = Heap::new(Expr::new(
        ExprKind::Binary {
            op: BinOp::And,
            left: Heap::new(Expr::ident(test_ident("c"))),
            right: not_d,
        },
        span,
    ));

    let or = Expr::new(
        ExprKind::Binary {
            op: BinOp::Or,
            left: and1,
            right: and2,
        },
        span,
    );

    // Verify structure
    match &or.kind {
        ExprKind::Binary {
            op: BinOp::Or,
            left,
            right,
        } => {
            assert!(matches!(left.kind, ExprKind::Binary { op: BinOp::And, .. }));
            assert!(matches!(
                right.kind,
                ExprKind::Binary { op: BinOp::And, .. }
            ));
        }
        _ => panic!("Expected OR at root"),
    }
}

#[test]
fn test_comparison_chain() {
    let span = test_span();

    // Build: a < b && b <= c && c != d
    let lt = Heap::new(Expr::new(
        ExprKind::Binary {
            op: BinOp::Lt,
            left: Heap::new(Expr::ident(test_ident("a"))),
            right: Heap::new(Expr::ident(test_ident("b"))),
        },
        span,
    ));

    let le = Heap::new(Expr::new(
        ExprKind::Binary {
            op: BinOp::Le,
            left: Heap::new(Expr::ident(test_ident("b"))),
            right: Heap::new(Expr::ident(test_ident("c"))),
        },
        span,
    ));

    let ne = Heap::new(Expr::new(
        ExprKind::Binary {
            op: BinOp::Ne,
            left: Heap::new(Expr::ident(test_ident("c"))),
            right: Heap::new(Expr::ident(test_ident("d"))),
        },
        span,
    ));

    let and1 = Heap::new(Expr::new(
        ExprKind::Binary {
            op: BinOp::And,
            left: lt,
            right: le,
        },
        span,
    ));

    let chain = Expr::new(
        ExprKind::Binary {
            op: BinOp::And,
            left: and1,
            right: ne,
        },
        span,
    );

    // Verify it's a chain of ANDs
    match &chain.kind {
        ExprKind::Binary {
            op: BinOp::And,
            left,
            right,
        } => {
            assert!(matches!(left.kind, ExprKind::Binary { op: BinOp::And, .. }));
            assert!(matches!(right.kind, ExprKind::Binary { op: BinOp::Ne, .. }));
        }
        _ => panic!("Expected AND chain"),
    }
}

#[test]
fn test_method_call_chain() {
    let span = test_span();

    // Build: obj.method1().method2(arg).method3()
    let call1 = Expr::new(
        ExprKind::MethodCall {
            receiver: Heap::new(Expr::ident(test_ident("obj"))),
            method: test_ident("method1"),
            args: List::new(),
            type_args: List::new(),
        },
        span,
    );

    let call2 = Expr::new(
        ExprKind::MethodCall {
            receiver: Heap::new(call1),
            method: test_ident("method2"),
            args: List::from(vec![Expr::ident(test_ident("arg"))]),
            type_args: List::new(),
        },
        span,
    );

    let call3 = Expr::new(
        ExprKind::MethodCall {
            receiver: Heap::new(call2),
            method: test_ident("method3"),
            args: List::new(),
            type_args: List::new(),
        },
        span,
    );

    // Verify it's a chain
    match &call3.kind {
        ExprKind::MethodCall {
            receiver, method, ..
        } => {
            assert_eq!(method.name.as_str(), "method3");
            assert!(matches!(receiver.kind, ExprKind::MethodCall { .. }));
        }
        _ => panic!("Expected method call"),
    }
}

#[test]
fn test_field_access_chain() {
    let span = test_span();

    // Build: obj.field1.field2.field3
    let access1 = Expr::new(
        ExprKind::Field {
            expr: Heap::new(Expr::ident(test_ident("obj"))),
            field: test_ident("field1"),
        },
        span,
    );

    let access2 = Expr::new(
        ExprKind::Field {
            expr: Heap::new(access1),
            field: test_ident("field2"),
        },
        span,
    );

    let access3 = Expr::new(
        ExprKind::Field {
            expr: Heap::new(access2),
            field: test_ident("field3"),
        },
        span,
    );

    // Verify chain
    match &access3.kind {
        ExprKind::Field { expr, field } => {
            assert_eq!(field.name.as_str(), "field3");
            assert!(matches!(expr.kind, ExprKind::Field { .. }));
        }
        _ => panic!("Expected field access"),
    }
}

#[test]
fn test_optional_chaining() {
    let span = test_span();

    // Build: obj?.field1?.field2
    let chain1 = Expr::new(
        ExprKind::OptionalChain {
            expr: Heap::new(Expr::ident(test_ident("obj"))),
            field: test_ident("field1"),
        },
        span,
    );

    let chain2 = Expr::new(
        ExprKind::OptionalChain {
            expr: Heap::new(chain1),
            field: test_ident("field2"),
        },
        span,
    );

    match &chain2.kind {
        ExprKind::OptionalChain { expr, field } => {
            assert_eq!(field.name.as_str(), "field2");
            assert!(matches!(expr.kind, ExprKind::OptionalChain { .. }));
        }
        _ => panic!("Expected optional chain"),
    }
}

#[test]
fn test_nested_if_else() {
    let span = test_span();

    // Build: if a { if b { 1 } else { 2 } } else { if c { 3 } else { 4 } }
    let inner_if1 = Expr::new(
        ExprKind::If {
            condition: Heap::new(IfCondition {
                conditions: smallvec![ConditionKind::Expr(Expr::ident(test_ident("b")))],
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

    let inner_if2 = Expr::new(
        ExprKind::If {
            condition: Heap::new(IfCondition {
                conditions: smallvec![ConditionKind::Expr(Expr::ident(test_ident("c")))],
                span,
            }),
            then_branch: Block {
                stmts: List::from(vec![Stmt::new(
                    StmtKind::Expr {
                        expr: Expr::literal(Literal::int(3, span)),
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
                            expr: Expr::literal(Literal::int(4, span)),
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

    let outer_if = Expr::new(
        ExprKind::If {
            condition: Heap::new(IfCondition {
                conditions: smallvec![ConditionKind::Expr(Expr::ident(test_ident("a")))],
                span,
            }),
            then_branch: Block {
                stmts: List::from(vec![Stmt::new(
                    StmtKind::Expr {
                        expr: inner_if1,
                        has_semi: true,
                    },
                    span,
                )]),
                expr: Maybe::None,
                span,
            },
            else_branch: Maybe::Some(Heap::new(inner_if2)),
        },
        span,
    );

    // Verify structure
    match &outer_if.kind {
        ExprKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            assert_eq!(then_branch.stmts.len(), 1);
            assert!(else_branch.is_some());
        }
        _ => panic!("Expected if expression"),
    }
}

#[test]
fn test_complex_match() {
    let span = test_span();

    // Build a match with various pattern types
    let match_expr = Expr::new(
        ExprKind::Match {
            expr: Heap::new(Expr::ident(test_ident("value"))),
            arms: List::from(vec![
                // Literal pattern
                MatchArm {
                    attributes: verum_common::List::new(),
                    pattern: Pattern::literal(Literal::int(0, span)),
                    guard: Maybe::None,
                    body: Heap::new(Expr::literal(Literal::string("zero".to_string().into(), span))),
                    with_clause: Maybe::None,
                    span,
                },
                // Tuple pattern with guard
                MatchArm {
                    attributes: verum_common::List::new(),
                    pattern: Pattern::new(
                        PatternKind::Tuple(List::from(vec![
                            Pattern::ident(test_ident("x"), false, span),
                            Pattern::ident(test_ident("y"), false, span),
                        ])),
                        span,
                    ),
                    guard: Maybe::Some(Heap::new(Expr::new(
                        ExprKind::Binary {
                            op: BinOp::Gt,
                            left: Heap::new(Expr::ident(test_ident("x"))),
                            right: Heap::new(Expr::ident(test_ident("y"))),
                        },
                        span,
                    ))),
                    body: Heap::new(Expr::literal(Literal::string("x > y".to_string().into(), span))),
                    with_clause: Maybe::None,
                    span,
                },
                // Wildcard pattern
                MatchArm {
                    attributes: verum_common::List::new(),
                    pattern: Pattern::wildcard(span),
                    guard: Maybe::None,
                    body: Heap::new(Expr::literal(Literal::string("other".to_string().into(), span))),
                    with_clause: Maybe::None,
                    span,
                },
            ]),
        },
        span,
    );

    match &match_expr.kind {
        ExprKind::Match { arms, .. } => {
            assert_eq!(arms.len(), 3);
            assert!(arms[1].guard.is_some());
        }
        _ => panic!("Expected match expression"),
    }
}

#[test]
fn test_nested_loops() {
    let span = test_span();

    // Build nested for loop
    let inner_loop = Expr::new(
        ExprKind::For {
            label: Maybe::None,
            pattern: Pattern::ident(test_ident("j"), false, span),
            iter: Heap::new(Expr::ident(test_ident("inner_range"))),
            body: Block {
                stmts: List::from(vec![Stmt::new(
                    StmtKind::Expr {
                        expr: Expr::ident(test_ident("process")),
                        has_semi: true,
                    },
                    span,
                )]),
                expr: Maybe::None,
                span,
            },
            invariants: List::new(),
            decreases: List::new(),
        },
        span,
    );

    let outer_loop = Expr::new(
        ExprKind::For {
            label: Maybe::None,
            pattern: Pattern::ident(test_ident("i"), false, span),
            iter: Heap::new(Expr::ident(test_ident("outer_range"))),
            body: Block {
                stmts: List::from(vec![Stmt::new(
                    StmtKind::Expr {
                        expr: inner_loop,
                        has_semi: true,
                    },
                    span,
                )]),
                expr: Maybe::None,
                span,
            },
            invariants: List::new(),
            decreases: List::new(),
        },
        span,
    );

    match &outer_loop.kind {
        ExprKind::For { body, .. } => {
            assert_eq!(body.stmts.len(), 1);
            match &body.stmts[0].kind {
                StmtKind::Expr { expr, .. } => {
                    assert!(matches!(expr.kind, ExprKind::For { .. }));
                }
                _ => panic!("Expected expression statement"),
            }
        }
        _ => panic!("Expected for loop"),
    }
}

#[test]
fn test_closure_with_captures() {
    let span = test_span();

    // Build: |x, y| x + y + captured
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
                    left: Heap::new(Expr::new(
                        ExprKind::Binary {
                            op: BinOp::Add,
                            left: Heap::new(Expr::ident(test_ident("x"))),
                            right: Heap::new(Expr::ident(test_ident("y"))),
                        },
                        span,
                    )),
                    right: Heap::new(Expr::ident(test_ident("captured"))),
                },
                span,
            )),
        },
        span,
    );

    match &closure.kind {
        ExprKind::Closure { params, body, .. } => {
            assert_eq!(params.len(), 2);
            assert!(matches!(body.kind, ExprKind::Binary { op: BinOp::Add, .. }));
        }
        _ => panic!("Expected closure"),
    }
}

#[test]
fn test_async_await_expressions() {
    let span = test_span();

    // Build: async { await foo(); await bar() }
    let await1 = Expr::new(
        ExprKind::Await(Heap::new(Expr::new(
            ExprKind::Call {
                func: Heap::new(Expr::ident(test_ident("foo"))),
                args: List::new(),
                type_args: List::new(),
            },
            span,
        ))),
        span,
    );

    let await2 = Expr::new(
        ExprKind::Await(Heap::new(Expr::new(
            ExprKind::Call {
                func: Heap::new(Expr::ident(test_ident("bar"))),
                args: List::new(),
                type_args: List::new(),
            },
            span,
        ))),
        span,
    );

    let async_block = Expr::new(
        ExprKind::Async(Block {
            stmts: List::from(vec![
                Stmt::new(
                    StmtKind::Expr {
                        expr: await1,
                        has_semi: true,
                    },
                    span,
                ),
                Stmt::new(
                    StmtKind::Expr {
                        expr: await2,
                        has_semi: true,
                    },
                    span,
                ),
            ]),
            expr: Maybe::None,
            span,
        }),
        span,
    );

    match &async_block.kind {
        ExprKind::Async(body) => {
            assert_eq!(body.stmts.len(), 2);
            for stmt in &body.stmts {
                match &stmt.kind {
                    StmtKind::Expr { expr, .. } => {
                        assert!(matches!(expr.kind, ExprKind::Await(_)));
                    }
                    _ => panic!("Expected expression statement"),
                }
            }
        }
        _ => panic!("Expected async block"),
    }
}

#[test]
fn test_stream_comprehension() {
    let span = test_span();

    // Build: stream [x * 2 for x in numbers if x > 0 for y in others if y < 10]
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
                ComprehensionClause {
                    kind: ComprehensionClauseKind::For {
                        pattern: Pattern::ident(test_ident("y"), false, span),
                        iter: Expr::ident(test_ident("others")),
                    },
                    span,
                },
                ComprehensionClause {
                    kind: ComprehensionClauseKind::If(Expr::new(
                        ExprKind::Binary {
                            op: BinOp::Lt,
                            left: Heap::new(Expr::ident(test_ident("y"))),
                            right: Heap::new(Expr::literal(Literal::int(10, span))),
                        },
                        span,
                    )),
                    span,
                },
            ]),
        },
        span,
    );

    match &comprehension.kind {
        ExprKind::StreamComprehension { clauses, .. } => {
            assert_eq!(clauses.len(), 4);
            // Check alternating for/if pattern
            assert!(matches!(
                clauses[0].kind,
                ComprehensionClauseKind::For { .. }
            ));
            assert!(matches!(
                clauses[1].kind,
                ComprehensionClauseKind::If { .. }
            ));
            assert!(matches!(
                clauses[2].kind,
                ComprehensionClauseKind::For { .. }
            ));
            assert!(matches!(
                clauses[3].kind,
                ComprehensionClauseKind::If { .. }
            ));
        }
        _ => panic!("Expected stream comprehension"),
    }
}

#[test]
fn test_pipeline_expression() {
    let span = test_span();

    // Build: data |> filter |> map |> reduce
    let pipe1 = Expr::new(
        ExprKind::Pipeline {
            left: Heap::new(Expr::ident(test_ident("data"))),
            right: Heap::new(Expr::ident(test_ident("filter"))),
        },
        span,
    );

    let pipe2 = Expr::new(
        ExprKind::Pipeline {
            left: Heap::new(pipe1),
            right: Heap::new(Expr::ident(test_ident("map"))),
        },
        span,
    );

    let pipe3 = Expr::new(
        ExprKind::Pipeline {
            left: Heap::new(pipe2),
            right: Heap::new(Expr::ident(test_ident("reduce"))),
        },
        span,
    );

    // Verify pipeline structure
    match &pipe3.kind {
        ExprKind::Pipeline { left, .. } => {
            assert!(matches!(left.kind, ExprKind::Pipeline { .. }));
        }
        _ => panic!("Expected pipeline expression"),
    }
}

#[test]
fn test_try_expression() {
    let span = test_span();

    // Build: risky_op()?
    let try_expr = Expr::new(
        ExprKind::Try(Heap::new(Expr::new(
            ExprKind::Call {
                func: Heap::new(Expr::ident(test_ident("risky_op"))),
                args: List::new(),
                type_args: List::new(),
            },
            span,
        ))),
        span,
    );

    // TryBlock doesn't exist in current AST - just verify Try expression
    match &try_expr.kind {
        ExprKind::Try(_) => {
            // Try expression created successfully
        }
        _ => panic!("Expected try expression"),
    }
}

#[test]
fn test_array_expressions() {
    let span = test_span();

    // Array with elements
    let array_elements = Expr::new(
        ExprKind::Array(ArrayExpr::List(List::from(vec![
            Expr::literal(Literal::int(1, span)),
            Expr::literal(Literal::int(2, span)),
            Expr::literal(Literal::int(3, span)),
        ]))),
        span,
    );

    match &array_elements.kind {
        ExprKind::Array(ArrayExpr::List(elems)) => {
            assert_eq!(elems.len(), 3);
        }
        _ => panic!("Expected array elements"),
    }

    // Array repeat: [0; 100]
    let array_repeat = Expr::new(
        ExprKind::Array(ArrayExpr::Repeat {
            value: Heap::new(Expr::literal(Literal::int(0, span))),
            count: Heap::new(Expr::literal(Literal::int(100, span))),
        }),
        span,
    );

    match &array_repeat.kind {
        ExprKind::Array(ArrayExpr::Repeat { .. }) => {}
        _ => panic!("Expected array repeat"),
    }
}

#[test]
fn test_index_expression() {
    let span = test_span();

    // Build: array[i][j]
    let index1 = Expr::new(
        ExprKind::Index {
            expr: Heap::new(Expr::ident(test_ident("array"))),
            index: Heap::new(Expr::ident(test_ident("i"))),
        },
        span,
    );

    let index2 = Expr::new(
        ExprKind::Index {
            expr: Heap::new(index1),
            index: Heap::new(Expr::ident(test_ident("j"))),
        },
        span,
    );

    match &index2.kind {
        ExprKind::Index { expr, .. } => {
            assert!(matches!(expr.kind, ExprKind::Index { .. }));
        }
        _ => panic!("Expected index expression"),
    }
}

#[test]
fn test_range_expressions() {
    let span = test_span();

    // Full range: start..end
    let full_range = Expr::new(
        ExprKind::Range {
            start: Maybe::Some(Heap::new(Expr::literal(Literal::int(0, span)))),
            end: Maybe::Some(Heap::new(Expr::literal(Literal::int(10, span)))),
            inclusive: false,
        },
        span,
    );

    // Inclusive range: start..=end
    let inclusive_range = Expr::new(
        ExprKind::Range {
            start: Maybe::Some(Heap::new(Expr::literal(Literal::int(0, span)))),
            end: Maybe::Some(Heap::new(Expr::literal(Literal::int(10, span)))),
            inclusive: true,
        },
        span,
    );

    // Half-open ranges
    let from_start = Expr::new(
        ExprKind::Range {
            start: Maybe::Some(Heap::new(Expr::literal(Literal::int(5, span)))),
            end: Maybe::None,
            inclusive: false,
        },
        span,
    );

    let to_end = Expr::new(
        ExprKind::Range {
            start: Maybe::None,
            end: Maybe::Some(Heap::new(Expr::literal(Literal::int(5, span)))),
            inclusive: false,
        },
        span,
    );

    // Full range: ..
    let full = Expr::new(
        ExprKind::Range {
            start: Maybe::None,
            end: Maybe::None,
            inclusive: false,
        },
        span,
    );

    // Verify all range types compile
    assert!(matches!(full_range.kind, ExprKind::Range { .. }));
    assert!(matches!(
        inclusive_range.kind,
        ExprKind::Range {
            inclusive: true,
            ..
        }
    ));
    assert!(matches!(
        from_start.kind,
        ExprKind::Range {
            end: Maybe::None,
            ..
        }
    ));
    assert!(matches!(
        to_end.kind,
        ExprKind::Range {
            start: Maybe::None,
            ..
        }
    ));
    assert!(matches!(
        full.kind,
        ExprKind::Range {
            start: Maybe::None,
            end: Maybe::None,
            ..
        }
    ));
}

#[test]
fn test_cast_expression() {
    let span = test_span();

    // Build: value as Int
    let cast = Expr::new(
        ExprKind::Cast {
            expr: Heap::new(Expr::ident(test_ident("value"))),
            ty: Type::int(span),
        },
        span,
    );

    match &cast.kind {
        ExprKind::Cast { ty, .. } => {
            assert_eq!(ty.kind, TypeKind::Int);
        }
        _ => panic!("Expected cast expression"),
    }
}

#[test]
fn test_reference_expressions() {
    let span = test_span();

    // Immutable reference: &value
    let ref_expr = Expr::new(
        ExprKind::Unary {
            op: UnOp::Ref,
            expr: Heap::new(Expr::ident(test_ident("value"))),
        },
        span,
    );

    // Mutable reference: &mut value
    let mut_ref = Expr::new(
        ExprKind::Unary {
            op: UnOp::RefMut,
            expr: Heap::new(Expr::ident(test_ident("value"))),
        },
        span,
    );

    assert!(matches!(
        ref_expr.kind,
        ExprKind::Unary { op: UnOp::Ref, .. }
    ));
    assert!(matches!(
        mut_ref.kind,
        ExprKind::Unary {
            op: UnOp::RefMut,
            ..
        }
    ));
}

// Property-based tests for expression trees
proptest! {
    #[test]
    fn prop_binary_expr_depth(depth in 1usize..10) {
        let span = test_span();
        let mut expr = Expr::literal(Literal::int(0, span));

        for i in 0..depth {
            expr = Expr::new(
                ExprKind::Binary {
                    op: if i % 2 == 0 { BinOp::Add } else { BinOp::Mul },
                    left: Heap::new(expr),
                    right: Heap::new(Expr::literal(Literal::int(i as i128, span))),
                },
                span,
            );
        }

        // Count the depth
        fn count_depth(expr: &Expr) -> usize {
            match &expr.kind {
                ExprKind::Binary { left, .. } => 1 + count_depth(left),
                _ => 0,
            }
        }

        assert_eq!(count_depth(&expr), depth);
    }

    #[test]
    fn prop_method_chain_length(length in 1usize..20) {
        let span = test_span();
        let mut expr = Expr::ident(test_ident("obj"));

        for i in 0..length {
            expr = Expr::new(
                ExprKind::MethodCall {
                    receiver: Heap::new(expr),
                    method: test_ident(&format!("method{}", i)),
                    args: List::new(),
                type_args: List::new(),
                },
                span,
            );
        }

        // Count chain length
        fn count_chain(expr: &Expr) -> usize {
            match &expr.kind {
                ExprKind::MethodCall { receiver, .. } => 1 + count_chain(receiver),
                _ => 0,
            }
        }

        assert_eq!(count_chain(&expr), length);
    }
}

#[test]
fn test_extremely_deep_expression() {
    let span = test_span();
    let mut expr = Expr::literal(Literal::int(0, span));

    // Create a very deep expression tree (100 levels)
    for i in 1..=100 {
        let op = match i % 4 {
            0 => BinOp::Add,
            1 => BinOp::Sub,
            2 => BinOp::Mul,
            _ => BinOp::Div,
        };

        expr = Expr::new(
            ExprKind::Binary {
                op,
                left: Heap::new(expr),
                right: Heap::new(Expr::literal(Literal::int(i, span))),
            },
            span,
        );
    }

    // Should be able to create and work with very deep trees
    fn count_nodes(expr: &Expr) -> usize {
        match &expr.kind {
            ExprKind::Binary { left, right, .. } => 1 + count_nodes(left) + count_nodes(right),
            _ => 1,
        }
    }

    assert!(count_nodes(&expr) > 100);
}
