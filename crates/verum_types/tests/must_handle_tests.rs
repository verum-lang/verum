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
//! Comprehensive tests for @must_handle annotation flow-sensitive analysis
//!
//! Error handling: Result<T, E> and Maybe<T> types, try (?) operator with automatic From conversion, error propagation — Section 2.6
//!
//! This test suite validates that the flow-sensitive control flow analysis correctly
//! enforces @must_handle annotation requirements across various code patterns.

use smallvec::SmallVec;
use verum_ast::MatchArm;
use verum_ast::expr::{BinOp, Block, ConditionKind, Expr, ExprKind, IfCondition};
use verum_ast::literal::Literal;
use verum_ast::pattern::{Pattern, PatternKind, VariantPatternData};
use verum_ast::span::Span;
use verum_ast::stmt::{Stmt, StmtKind};
use verum_ast::ty::{Ident, Path, PathSegment, Type as AstType};
use verum_common::{Heap, List, Text};
use verum_types::{FlowSensitiveChecker, MustHandleRegistry, ResultState, Type, TypeError};

/// Helper to create a span for testing
fn test_span(_line: usize, col: usize) -> Span {
    Span::new(
        col as u32,
        (col + 10) as u32,
        verum_ast::span::FileId::dummy(),
    )
}

/// Helper to create an identifier
fn ident(name: &str) -> Ident {
    Ident {
        name: Text::from(name),
        span: test_span(1, 1),
    }
}

/// Helper to create an IfCondition from an expression
fn if_cond(expr: Expr) -> Heap<IfCondition> {
    Heap::new(IfCondition {
        conditions: SmallVec::from_vec(vec![ConditionKind::Expr(expr)]),
        span: test_span(1, 1),
    })
}

/// Helper to create a Result<T, E> type
fn result_type(ok_type: &str, err_type: &str) -> Type {
    let result_ident = Ident::new("Result", Span::dummy());
    let result_path = Path::from_ident(result_ident);

    let ok_ident = Ident::new(ok_type, Span::dummy());
    let ok_path = Path::from_ident(ok_ident);
    let ok_ty = Type::Named {
        path: ok_path,
        args: vec![].into(),
    };

    let err_ident = Ident::new(err_type, Span::dummy());
    let err_path = Path::from_ident(err_ident);
    let err_ty = Type::Named {
        path: err_path,
        args: vec![].into(),
    };

    Type::Named {
        path: result_path,
        args: vec![ok_ty, err_ty].into(),
    }
}

/// Helper to create an expression
fn expr(kind: ExprKind) -> Expr {
    Expr {
        kind,
        span: test_span(1, 1),
        ref_kind: None,
        check_eliminated: false,
    }
}

#[test]
fn test_1_direct_drop_error() {
    // Test: let _ = result → E0317
    // @must_handle type CriticalError
    // fn bad() { let _ = risky(); }  // ❌ E0317

    let mut registry = MustHandleRegistry::new();
    registry.register("CriticalError");

    let mut checker = FlowSensitiveChecker::new(registry);

    // Create expression: let _ = risky()
    let body = expr(ExprKind::Block(Block::new(
        vec![Stmt::new(
            StmtKind::Let {
                pattern: Pattern::wildcard(test_span(1, 1)),
                ty: None,
                value: Some(expr(ExprKind::Call { type_args: vec![].into(),
                    func: Box::new(expr(ExprKind::Path(Path::single(ident("risky"))))),
                    args: List::new(),
                })),
            },
            test_span(1, 1),
        )]
        .into(),
        None,
        test_span(1, 1),
    )));

    // Should fail because Result is ignored
    let result = checker.analyze(&body);
    assert!(result.is_err(), "Expected E0317 error for wildcard pattern");

    if let Err(TypeError::Other(msg)) = result {
        assert!(
            msg.contains("unused Result"),
            "Error should mention unused Result"
        );
        assert!(
            msg.contains("CriticalError"),
            "Error should mention the error type"
        );
    }
}

#[test]
fn test_2_try_operator_ok() {
    // Test: result? → OK
    // fn good() -> Result<(), E> { let x = risky()?; Ok(()) }  // ✅ OK

    let mut registry = MustHandleRegistry::new();
    registry.register("CriticalError");

    let mut checker = FlowSensitiveChecker::new(registry);

    // Create expression: let x = risky()?
    let body = expr(ExprKind::Block(Block::new(
        vec![Stmt::new(
            StmtKind::Let {
                pattern: Pattern::ident(ident("x"), false, test_span(1, 1)),
                ty: None,
                value: Some(expr(ExprKind::Try(Box::new(expr(ExprKind::Call { type_args: vec![].into(),
                    func: Box::new(expr(ExprKind::Path(Path::single(ident("risky"))))),
                    args: List::new(),
                }))))),
            },
            test_span(1, 1),
        )]
        .into(),
        Some(Box::new(expr(ExprKind::Path(Path::single(ident("Ok")))))),
        test_span(1, 1),
    )));

    // Should succeed because ? operator handles the Result
    let result = checker.analyze(&body);
    assert!(
        result.is_ok(),
        "? operator should handle the Result: {:?}",
        result
    );
}

#[test]
fn test_3_unwrap_ok() {
    // Test: result.unwrap() → OK
    // fn good() { risky().unwrap(); }  // ✅ OK

    let mut registry = MustHandleRegistry::new();
    registry.register("CriticalError");

    let mut checker = FlowSensitiveChecker::new(registry);

    // Create expression: risky().unwrap()
    let body = expr(ExprKind::MethodCall { type_args: vec![].into(),
        receiver: Box::new(expr(ExprKind::Call { type_args: vec![].into(),
            func: Box::new(expr(ExprKind::Path(Path::single(ident("risky"))))),
            args: List::new(),
        })),
        method: ident("unwrap"),
        args: List::new(),
    });

    let result = checker.analyze(&body);
    assert!(
        result.is_ok(),
        "unwrap() should handle the Result: {:?}",
        result
    );
}

#[test]
fn test_4_match_ok() {
    // Test: match result { ... } → OK
    // fn good() { match risky() { Ok(x) => {}, Err(e) => {} } }  // ✅ OK

    let mut registry = MustHandleRegistry::new();
    registry.register("CriticalError");

    let mut checker = FlowSensitiveChecker::new(registry);

    // Create expression: match risky() { Ok(x) => ..., Err(e) => ... }
    let body = expr(ExprKind::Match {
        expr: Box::new(expr(ExprKind::Call { type_args: vec![].into(),
            func: Box::new(expr(ExprKind::Path(Path::single(ident("risky"))))),
            args: List::new(),
        })),
        arms: vec![
            verum_ast::pattern::MatchArm {
                attributes: verum_common::List::new(),
                pattern: Pattern::new(
                    PatternKind::Variant {
                        path: Path::single(ident("Ok")),
                        data: Some(VariantPatternData::Tuple(
                            vec![Pattern::ident(ident("x"), false, test_span(1, 1))].into(),
                        )),
                    },
                    test_span(1, 1),
                ),
                guard: None,
                body: Box::new(expr(ExprKind::Block(Block::empty(test_span(1, 1))))),
                with_clause: verum_common::Maybe::None,
                span: test_span(1, 1),
            },
            verum_ast::pattern::MatchArm {
                attributes: verum_common::List::new(),
                pattern: Pattern::new(
                    PatternKind::Variant {
                        path: Path::single(ident("Err")),
                        data: Some(VariantPatternData::Tuple(
                            vec![Pattern::ident(ident("e"), false, test_span(1, 1))].into(),
                        )),
                    },
                    test_span(1, 1),
                ),
                guard: None,
                body: Box::new(expr(ExprKind::Block(Block::empty(test_span(1, 1))))),
                with_clause: verum_common::Maybe::None,
                span: test_span(1, 1),
            },
        ]
        .into(),
    });

    let result = checker.analyze(&body);
    assert!(
        result.is_ok(),
        "match expression should handle the Result: {:?}",
        result
    );
}

#[test]
fn test_5_is_err_check_ok() {
    // Test: if result.is_err() { drop(result) } → OK
    // fn good() {
    //     let result = risky();
    //     if result.is_err() { /* checked */ }
    // }  // ✅ OK

    let mut registry = MustHandleRegistry::new();
    registry.register("CriticalError");

    let mut checker = FlowSensitiveChecker::new(registry);

    // Create expression:
    // let result = risky();
    // if result.is_err() { }
    let body = expr(ExprKind::Block(Block::new(
        vec![
            Stmt::new(
                StmtKind::Let {
                    pattern: Pattern::ident(ident("result"), false, test_span(1, 1)),
                    ty: None,
                    value: Some(expr(ExprKind::Call { type_args: vec![].into(),
                        func: Box::new(expr(ExprKind::Path(Path::single(ident("risky"))))),
                        args: List::new(),
                    })),
                },
                test_span(1, 1),
            ),
            Stmt::expr(
                expr(ExprKind::If {
                    condition: if_cond(expr(ExprKind::MethodCall { type_args: vec![].into(),
                        receiver: Box::new(expr(ExprKind::Path(Path::single(ident("result"))))),
                        method: ident("is_err"),
                        args: List::new(),
                    })),
                    then_branch: Block::empty(test_span(1, 1)),
                    else_branch: None,
                }),
                false,
            ),
        ]
        .into(),
        None,
        test_span(1, 1),
    )));

    let result = checker.analyze(&body);
    assert!(
        result.is_ok(),
        "is_err() check should mark Result as safe: {:?}",
        result
    );
}

#[test]
fn test_6_conditional_branches_both_handle() {
    // Test: Both branches handle → OK
    // fn good() {
    //     let result = risky();
    //     if condition {
    //         result.unwrap();  // Handled
    //     } else {
    //         result.unwrap();  // Handled
    //     }
    // }  // ✅ OK

    let mut registry = MustHandleRegistry::new();
    registry.register("CriticalError");

    let mut checker = FlowSensitiveChecker::new(registry);

    let body = expr(ExprKind::Block(Block::new(
        vec![
            Stmt::new(
                StmtKind::Let {
                    pattern: Pattern::ident(ident("result"), false, test_span(1, 1)),
                    ty: None,
                    value: Some(expr(ExprKind::Call { type_args: vec![].into(),
                        func: Box::new(expr(ExprKind::Path(Path::single(ident("risky"))))),
                        args: List::new(),
                    })),
                },
                test_span(1, 1),
            ),
            Stmt::expr(
                expr(ExprKind::If {
                    condition: if_cond(expr(ExprKind::Path(Path::single(ident("condition"))))),
                    then_branch: Block::new(
                        vec![].into(),
                        Some(Box::new(expr(ExprKind::MethodCall { type_args: vec![].into(),
                            receiver: Box::new(expr(ExprKind::Path(Path::single(ident("result"))))),
                            method: ident("unwrap"),
                            args: List::new(),
                        }))),
                        test_span(1, 1),
                    ),
                    else_branch: Some(Box::new(expr(ExprKind::MethodCall { type_args: vec![].into(),
                        receiver: Box::new(expr(ExprKind::Path(Path::single(ident("result"))))),
                        method: ident("unwrap"),
                        args: List::new(),
                    }))),
                }),
                false,
            ),
        ]
        .into(),
        None,
        test_span(1, 1),
    )));

    let result = checker.analyze(&body);
    assert!(
        result.is_ok(),
        "Both branches handle → should be OK: {:?}",
        result
    );
}

#[test]
fn test_7_conditional_branches_partial_handle() {
    // Test: Only one branch handles → E0317
    // fn bad() {
    //     let result = risky();
    //     if condition {
    //         result.unwrap();  // Handled
    //     }
    //     // else branch: not handled
    // }  // ❌ E0317

    let mut registry = MustHandleRegistry::new();
    registry.register("CriticalError");

    let mut checker = FlowSensitiveChecker::new(registry);

    let body = expr(ExprKind::Block(Block::new(
        vec![
            Stmt::new(
                StmtKind::Let {
                    pattern: Pattern::ident(ident("result"), false, test_span(1, 1)),
                    ty: None,
                    value: Some(expr(ExprKind::Call { type_args: vec![].into(),
                        func: Box::new(expr(ExprKind::Path(Path::single(ident("risky"))))),
                        args: List::new(),
                    })),
                },
                test_span(1, 1),
            ),
            Stmt::expr(
                expr(ExprKind::If {
                    condition: if_cond(expr(ExprKind::Path(Path::single(ident("condition"))))),
                    then_branch: Block::new(
                        vec![].into(),
                        Some(Box::new(expr(ExprKind::MethodCall { type_args: vec![].into(),
                            receiver: Box::new(expr(ExprKind::Path(Path::single(ident("result"))))),
                            method: ident("unwrap"),
                            args: List::new(),
                        }))),
                        test_span(1, 1),
                    ),
                    else_branch: None, // Not handled in else
                }),
                false,
            ),
        ]
        .into(),
        None,
        test_span(1, 1),
    )));

    let result = checker.analyze(&body);
    assert!(
        result.is_err(),
        "Only one branch handles → should fail with E0317"
    );
}

#[test]
fn test_8_loop_with_early_return() {
    // Test: Loop with early return → OK
    // fn good() -> Result<(), E> {
    //     loop {
    //         let result = risky()?;  // Handled via ?
    //         if done { return Ok(()); }
    //     }
    // }  // ✅ OK

    let mut registry = MustHandleRegistry::new();
    registry.register("CriticalError");

    let mut checker = FlowSensitiveChecker::new(registry);

    let body = expr(ExprKind::Loop {
        label: verum_common::Maybe::None,
        body: Block::new(
            vec![
                Stmt::new(
                    StmtKind::Let {
                        pattern: Pattern::ident(ident("result"), false, test_span(1, 1)),
                        ty: verum_common::Maybe::None,
                        value: verum_common::Maybe::Some(expr(ExprKind::Try(Box::new(expr(ExprKind::Call { type_args: vec![].into(),
                            func: Box::new(expr(ExprKind::Path(Path::single(ident("risky"))))),
                            args: List::new(),
                        }))))),
                    },
                    test_span(1, 1),
                ),
                Stmt::expr(
                    expr(ExprKind::If {
                        condition: if_cond(expr(ExprKind::Path(Path::single(ident("done"))))),
                        then_branch: Block::new(
                            vec![].into(),
                            verum_common::Maybe::Some(Box::new(expr(ExprKind::Return(verum_common::Maybe::Some(Box::new(expr(
                                ExprKind::Path(Path::single(ident("Ok"))),
                            ))))))),
                            test_span(1, 1),
                        ),
                        else_branch: verum_common::Maybe::None,
                    }),
                    false,
                ),
            ]
            .into(),
            verum_common::Maybe::None,
            test_span(1, 1),
        ),
        invariants: vec![].into(),
    });

    let result = checker.analyze(&body);
    assert!(
        result.is_ok(),
        "Loop with ? operator should handle Result: {:?}",
        result
    );
}

#[test]
fn test_9_nested_scopes() {
    // Test: Nested scopes with proper handling → OK
    // fn good() {
    //     {
    //         let result = risky();
    //         result.unwrap();
    //     }
    // }  // ✅ OK

    let mut registry = MustHandleRegistry::new();
    registry.register("CriticalError");

    let mut checker = FlowSensitiveChecker::new(registry);

    let body = expr(ExprKind::Block(Block::new(
        vec![Stmt::expr(
            expr(ExprKind::Block(Block::new(
                vec![
                    Stmt::new(
                        StmtKind::Let {
                            pattern: Pattern::ident(ident("result"), false, test_span(1, 1)),
                            ty: None,
                            value: Some(expr(ExprKind::Call { type_args: vec![].into(),
                                func: Box::new(expr(ExprKind::Path(Path::single(ident("risky"))))),
                                args: List::new(),
                            })),
                        },
                        test_span(1, 1),
                    ),
                    Stmt::expr(
                        expr(ExprKind::MethodCall { type_args: vec![].into(),
                            receiver: Box::new(expr(ExprKind::Path(Path::single(ident("result"))))),
                            method: ident("unwrap"),
                            args: List::new(),
                        }),
                        false,
                    ),
                ]
                .into(),
                None,
                test_span(1, 1),
            ))),
            false,
        )]
        .into(),
        None,
        test_span(1, 1),
    )));

    let result = checker.analyze(&body);
    assert!(
        result.is_ok(),
        "Nested scope with unwrap should be OK: {:?}",
        result
    );
}

#[test]
fn test_10_multiple_results() {
    // Test: Multiple Results, all handled → OK
    // fn good() {
    //     let r1 = risky();
    //     let r2 = risky();
    //     r1.unwrap();
    //     r2.unwrap();
    // }  // ✅ OK

    let mut registry = MustHandleRegistry::new();
    registry.register("CriticalError");

    let mut checker = FlowSensitiveChecker::new(registry);

    let body = expr(ExprKind::Block(Block::new(
        vec![
            Stmt::new(
                StmtKind::Let {
                    pattern: Pattern::ident(ident("r1"), false, test_span(1, 1)),
                    ty: None,
                    value: Some(expr(ExprKind::Call { type_args: vec![].into(),
                        func: Box::new(expr(ExprKind::Path(Path::single(ident("risky"))))),
                        args: List::new(),
                    })),
                },
                test_span(1, 1),
            ),
            Stmt::new(
                StmtKind::Let {
                    pattern: Pattern::ident(ident("r2"), false, test_span(1, 1)),
                    ty: None,
                    value: Some(expr(ExprKind::Call { type_args: vec![].into(),
                        func: Box::new(expr(ExprKind::Path(Path::single(ident("risky"))))),
                        args: List::new(),
                    })),
                },
                test_span(1, 1),
            ),
            Stmt::expr(
                expr(ExprKind::MethodCall { type_args: vec![].into(),
                    receiver: Box::new(expr(ExprKind::Path(Path::single(ident("r1"))))),
                    method: ident("unwrap"),
                    args: List::new(),
                }),
                false,
            ),
            Stmt::expr(
                expr(ExprKind::MethodCall { type_args: vec![].into(),
                    receiver: Box::new(expr(ExprKind::Path(Path::single(ident("r2"))))),
                    method: ident("unwrap"),
                    args: List::new(),
                }),
                false,
            ),
        ]
        .into(),
        None,
        test_span(1, 1),
    )));

    let result = checker.analyze(&body);
    assert!(
        result.is_ok(),
        "Multiple Results all handled should be OK: {:?}",
        result
    );
}

#[test]
fn test_11_multiple_results_one_unhandled() {
    // Test: Multiple Results, one not handled → E0317
    // fn bad() {
    //     let r1 = risky();
    //     let r2 = risky();
    //     r1.unwrap();
    //     // r2 not handled
    // }  // ❌ E0317

    let mut registry = MustHandleRegistry::new();
    registry.register("CriticalError");

    let mut checker = FlowSensitiveChecker::new(registry);

    let body = expr(ExprKind::Block(Block::new(
        vec![
            Stmt::new(
                StmtKind::Let {
                    pattern: Pattern::ident(ident("r1"), false, test_span(1, 1)),
                    ty: None,
                    value: Some(expr(ExprKind::Call { type_args: vec![].into(),
                        func: Box::new(expr(ExprKind::Path(Path::single(ident("risky"))))),
                        args: List::new(),
                    })),
                },
                test_span(1, 1),
            ),
            Stmt::new(
                StmtKind::Let {
                    pattern: Pattern::ident(ident("r2"), false, test_span(1, 1)),
                    ty: None,
                    value: Some(expr(ExprKind::Call { type_args: vec![].into(),
                        func: Box::new(expr(ExprKind::Path(Path::single(ident("risky"))))),
                        args: List::new(),
                    })),
                },
                test_span(1, 1),
            ),
            Stmt::expr(
                expr(ExprKind::MethodCall { type_args: vec![].into(),
                    receiver: Box::new(expr(ExprKind::Path(Path::single(ident("r1"))))),
                    method: ident("unwrap"),
                    args: List::new(),
                }),
                false,
            ),
        ]
        .into(),
        None,
        test_span(1, 1),
    )));

    let result = checker.analyze(&body);
    assert!(
        result.is_err(),
        "Multiple Results with one unhandled should fail: {:?}",
        result
    );
}

#[test]
fn test_12_expect_ok() {
    // Test: result.expect("msg") → OK
    // fn good() { risky().expect("critical operation failed"); }  // ✅ OK

    let mut registry = MustHandleRegistry::new();
    registry.register("CriticalError");

    let mut checker = FlowSensitiveChecker::new(registry);

    let body = expr(ExprKind::MethodCall { type_args: vec![].into(),
        receiver: Box::new(expr(ExprKind::Call { type_args: vec![].into(),
            func: Box::new(expr(ExprKind::Path(Path::single(ident("risky"))))),
            args: List::new(),
        })),
        method: ident("expect"),
        args: vec![expr(ExprKind::Literal(Literal::string(
            "failed".into(),
            test_span(1, 1),
        )))]
        .into(),
    });

    let result = checker.analyze(&body);
    assert!(
        result.is_ok(),
        "expect() should handle the Result: {:?}",
        result
    );
}

#[test]
fn test_13_non_must_handle_result_ignored_ok() {
    // Test: Non-@must_handle Result can be ignored
    // type RegularError is | NotFound;
    // fn regular() -> Result<Data, RegularError> { ... }
    // fn ok() { let _ = regular(); }  // ✅ OK (not @must_handle)

    let registry = MustHandleRegistry::new(); // Don't register RegularError

    let mut checker = FlowSensitiveChecker::new(registry);

    let body = expr(ExprKind::Block(Block::new(
        vec![Stmt::new(
            StmtKind::Let {
                pattern: Pattern::wildcard(test_span(1, 1)),
                ty: None,
                value: Some(expr(ExprKind::Call { type_args: vec![].into(),
                    func: Box::new(expr(ExprKind::Path(Path::single(ident("regular"))))),
                    args: List::new(),
                })),
            },
            test_span(1, 1),
        )]
        .into(),
        None,
        test_span(1, 1),
    )));

    let result = checker.analyze(&body);
    // Should be OK because RegularError is not @must_handle
    assert!(
        result.is_ok(),
        "Non-@must_handle Result can be ignored: {:?}",
        result
    );
}

#[test]
fn test_14_result_state_join() {
    // Test ResultState::join logic
    use ResultState::*;

    // Both handled → Handled
    assert_eq!(Handled.join(Handled), Handled);

    // Mixed handled/checked → Handled
    assert_eq!(Handled.join(Checked), Handled);
    assert_eq!(Checked.join(Handled), Handled);

    // Both checked → Checked
    assert_eq!(Checked.join(Checked), Checked);

    // Any unhandled → Unhandled (conservative)
    assert_eq!(Handled.join(Unhandled), Unhandled);
    assert_eq!(Unhandled.join(Handled), Unhandled);
    assert_eq!(Checked.join(Unhandled), Unhandled);
    assert_eq!(Unhandled.join(Checked), Unhandled);
    assert_eq!(Unhandled.join(Unhandled), Unhandled);
}

#[test]
fn test_15_is_must_handle_result_detection() {
    // Test type detection for Result<T, E> where E is @must_handle

    let mut registry = MustHandleRegistry::new();
    registry.register("CriticalError");

    let checker = FlowSensitiveChecker::new(registry);

    // Test Result<Data, CriticalError>
    let result_ty = result_type("Data", "CriticalError");
    assert!(
        checker.is_must_handle_result(&result_ty).is_some(),
        "Should detect @must_handle Result"
    );

    // Test Result<Data, RegularError> (not @must_handle)
    let regular_ty = result_type("Data", "RegularError");
    assert!(
        checker.is_must_handle_result(&regular_ty).is_none(),
        "Should not detect non-@must_handle Result"
    );

    // Test non-Result type
    let data_ident = Ident::new("Data", Span::dummy());
    let data_path = Path::from_ident(data_ident);
    let non_result = Type::Named {
        path: data_path,
        args: vec![].into(),
    };
    assert!(
        checker.is_must_handle_result(&non_result).is_none(),
        "Should not detect non-Result type"
    );
}

#[test]
fn test_16_if_let_ok_pattern() {
    // Test: if let Ok(x) = result { ... } → OK
    // fn good() {
    //     if let Ok(x) = risky() {
    //         use(x);
    //     }
    // }  // ✅ OK

    let mut registry = MustHandleRegistry::new();
    registry.register("CriticalError");

    let mut checker = FlowSensitiveChecker::new(registry);

    let body = expr(ExprKind::If {
        condition: Box::new(verum_ast::expr::IfCondition {
            conditions: smallvec::smallvec![verum_ast::expr::ConditionKind::Let {
                pattern: Pattern::new(
                    PatternKind::Variant {
                        path: Path::single(ident("Ok")),
                        data: Some(VariantPatternData::Tuple(
                            vec![Pattern::ident(ident("x"), false, test_span(1, 1))].into(),
                        )),
                    },
                    test_span(1, 1),
                ),
                value: expr(ExprKind::Call { type_args: vec![].into(),
                    func: Box::new(expr(ExprKind::Path(Path::single(ident("risky"))))),
                    args: List::new(),
                }),
            }],
            span: test_span(1, 1),
        }),
        then_branch: Block::new(
            vec![].into(),
            Some(Box::new(expr(ExprKind::Call { type_args: vec![].into(),
                func: Box::new(expr(ExprKind::Path(Path::single(ident("use"))))),
                args: vec![expr(ExprKind::Path(Path::single(ident("x"))))].into(),
            }))),
            test_span(1, 1),
        ),
        else_branch: None,
    });

    let result = checker.analyze(&body);
    assert!(
        result.is_ok(),
        "if let Ok pattern should handle Result: {:?}",
        result
    );
}

#[test]
fn test_17_cfg_basic_block_structure() {
    // Test CFG construction basics
    use verum_types::ControlFlowGraph;

    let mut cfg = ControlFlowGraph::new();

    // Entry and exit blocks should exist
    assert_eq!(cfg.blocks.len(), 2, "CFG should have entry and exit blocks");

    // Test block allocation
    let block1 = cfg.alloc_block();
    let block2 = cfg.alloc_block();
    assert_ne!(block1, block2, "Each allocation should get unique ID");

    // Test variable allocation
    let var1 = cfg.alloc_var("x");
    let var2 = cfg.alloc_var("y");
    let var3 = cfg.alloc_var("x"); // Duplicate name
    assert_eq!(var1, var3, "Same name should get same VarId");
    assert_ne!(var1, var2, "Different names should get different VarIds");
}
