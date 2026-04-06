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
// Tests for predicate declarations.
//
// This module tests PredicateDecl, which allows defining reusable
// boolean expressions for refinement types.
//
// Tests for refinement predicate construction and the five binding rules.

use verum_ast::decl::*;
use verum_ast::expr::{BinOp, Expr, ExprKind, UnOp};
use verum_ast::literal::Literal;
use verum_ast::pattern::Pattern;
use verum_ast::*;
use verum_common::{Heap, List, Maybe};

/// Helper function to create a test span
fn test_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

/// Helper function to create a test identifier
fn test_ident(name: &str) -> Ident {
    Ident::new(name.to_string(), test_span())
}

#[test]
fn test_predicate_decl_basic() {
    // predicate NonZero(x: Int) -> Bool { x != 0 }
    let span = test_span();

    let predicate = PredicateDecl {
        visibility: Visibility::Public,
        name: test_ident("NonZero"),
        generics: List::new(),
        params: List::from(vec![FunctionParam::new(
            FunctionParamKind::Regular {
                pattern: Pattern::ident(test_ident("x"), false, span),
                ty: Type::int(span),
                default_value: Maybe::None,
            },
            span,
        )]),
        return_type: Type::bool(span),
        body: Heap::new(Expr::new(
            ExprKind::Binary {
                left: Heap::new(Expr::new(
                    ExprKind::Path(Path::single(test_ident("x"))),
                    span,
                )),
                op: BinOp::Ne,
                right: Heap::new(Expr::literal(Literal::int(0, span))),
            },
            span,
        )),
        span,
    };

    assert_eq!(predicate.name.name.as_str(), "NonZero");
    assert_eq!(predicate.params.len(), 1);
    assert_eq!(predicate.return_type.kind, TypeKind::Bool);
    assert!(predicate.visibility.is_public());
}

#[test]
fn test_predicate_decl_positive() {
    // predicate Positive(x: Float) -> Bool { x > 0.0 }
    let span = test_span();

    let predicate = PredicateDecl {
        visibility: Visibility::Private,
        name: test_ident("Positive"),
        generics: List::new(),
        params: List::from(vec![FunctionParam {
            kind: FunctionParamKind::Regular {
                pattern: Pattern::ident(test_ident("x"), false, span),
                ty: Type::float(span),
                default_value: Maybe::None,
            },
            attributes: List::new(),
            span,
        }]),
        return_type: Type::bool(span),
        body: Heap::new(Expr::new(
            ExprKind::Binary {
                left: Heap::new(Expr::new(
                    ExprKind::Path(Path::single(test_ident("x"))),
                    span,
                )),
                op: BinOp::Gt,
                right: Heap::new(Expr::literal(Literal::float(0.0, span))),
            },
            span,
        )),
        span,
    };

    assert_eq!(predicate.name.name.as_str(), "Positive");
    assert!(!predicate.visibility.is_public());
}

#[test]
fn test_predicate_decl_multiple_params() {
    // predicate InRange(x: Int, min: Int, max: Int) -> Bool { x >= min && x <= max }
    let span = test_span();

    let left_cond = Expr::new(
        ExprKind::Binary {
            left: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("x"))),
                span,
            )),
            op: BinOp::Ge,
            right: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("min"))),
                span,
            )),
        },
        span,
    );

    let right_cond = Expr::new(
        ExprKind::Binary {
            left: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("x"))),
                span,
            )),
            op: BinOp::Le,
            right: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("max"))),
                span,
            )),
        },
        span,
    );

    let body = Expr::new(
        ExprKind::Binary {
            left: Heap::new(left_cond),
            op: BinOp::And,
            right: Heap::new(right_cond),
        },
        span,
    );

    let predicate = PredicateDecl {
        visibility: Visibility::Public,
        name: test_ident("InRange"),
        generics: List::new(),
        params: List::from(vec![
            FunctionParam::new(
                FunctionParamKind::Regular {
                    pattern: Pattern::ident(test_ident("x"), false, span),
                    ty: Type::int(span),
                    default_value: Maybe::None,
                },
                span,
            ),
            FunctionParam::new(
                FunctionParamKind::Regular {
                    pattern: Pattern::ident(test_ident("min"), false, span),
                    ty: Type::int(span),
                    default_value: Maybe::None,
                },
                span,
            ),
            FunctionParam::new(
                FunctionParamKind::Regular {
                    pattern: Pattern::ident(test_ident("max"), false, span),
                    ty: Type::int(span),
                    default_value: Maybe::None,
                },
                span,
            ),
        ]),
        return_type: Type::bool(span),
        body: Heap::new(body),
        span,
    };

    assert_eq!(predicate.params.len(), 3);
    match &predicate.body.kind {
        ExprKind::Binary { op, .. } => {
            assert_eq!(*op, BinOp::And);
        }
        _ => panic!("Expected binary expression"),
    }
}

#[test]
fn test_predicate_decl_complex_expression() {
    // predicate IsEven(x: Int) -> Bool { x % 2 == 0 }
    let span = test_span();

    let mod_expr = Expr::new(
        ExprKind::Binary {
            left: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("x"))),
                span,
            )),
            op: BinOp::Rem,
            right: Heap::new(Expr::literal(Literal::int(2, span))),
        },
        span,
    );

    let body = Expr::new(
        ExprKind::Binary {
            left: Heap::new(mod_expr),
            op: BinOp::Eq,
            right: Heap::new(Expr::literal(Literal::int(0, span))),
        },
        span,
    );

    let predicate = PredicateDecl {
        visibility: Visibility::Public,
        name: test_ident("IsEven"),
        generics: List::new(),
        params: List::from(vec![FunctionParam::new(
            FunctionParamKind::Regular {
                pattern: Pattern::ident(test_ident("x"), false, span),
                ty: Type::int(span),
                default_value: Maybe::None,
            },
            span,
        )]),
        return_type: Type::bool(span),
        body: Heap::new(body),
        span,
    };

    assert_eq!(predicate.name.name.as_str(), "IsEven");
}

#[test]
fn test_predicate_decl_text_validation() {
    // predicate IsEmail(email: Text) -> Bool { email.contains("@") }
    let span = test_span();

    let body = Expr::new(
        ExprKind::MethodCall {
            receiver: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("email"))),
                span,
            )),
            method: test_ident("contains"),
            args: List::from(vec![Expr::literal(Literal::string("@".to_string().into(), span))]),
            type_args: List::new(),
        },
        span,
    );

    let predicate = PredicateDecl {
        visibility: Visibility::Public,
        name: test_ident("IsEmail"),
        generics: List::new(),
        params: List::from(vec![FunctionParam {
            kind: FunctionParamKind::Regular {
                pattern: Pattern::ident(test_ident("email"), false, span),
                ty: Type::text(span),
                default_value: Maybe::None,
            },
            attributes: List::new(),
            span,
        }]),
        return_type: Type::bool(span),
        body: Heap::new(body),
        span,
    };

    assert_eq!(predicate.name.name.as_str(), "IsEmail");
    assert!(matches!(predicate.body.kind, ExprKind::MethodCall { .. }));
}

#[test]
fn test_predicate_decl_item_kind() {
    // Test that PredicateDecl can be wrapped in ItemKind
    let span = test_span();

    let predicate = PredicateDecl {
        visibility: Visibility::Public,
        name: test_ident("IsPositive"),
        generics: List::new(),
        params: List::from(vec![FunctionParam::new(
            FunctionParamKind::Regular {
                pattern: Pattern::ident(test_ident("x"), false, span),
                ty: Type::int(span),
                default_value: Maybe::None,
            },
            span,
        )]),
        return_type: Type::bool(span),
        body: Heap::new(Expr::new(
            ExprKind::Binary {
                left: Heap::new(Expr::new(
                    ExprKind::Path(Path::single(test_ident("x"))),
                    span,
                )),
                op: BinOp::Gt,
                right: Heap::new(Expr::literal(Literal::int(0, span))),
            },
            span,
        )),
        span,
    };

    let item = Item::new(ItemKind::Predicate(predicate), span);

    match &item.kind {
        ItemKind::Predicate(p) => {
            assert_eq!(p.name.name.as_str(), "IsPositive");
        }
        _ => panic!("Expected Predicate item"),
    }
}

#[test]
fn test_predicate_decl_spanned() {
    let span = test_span();

    let predicate = PredicateDecl {
        visibility: Visibility::Public,
        name: test_ident("Test"),
        generics: List::new(),
        params: List::new(),
        return_type: Type::bool(span),
        body: Heap::new(Expr::literal(Literal::bool(true, span))),
        span,
    };

    assert_eq!(predicate.span(), span);
}

#[test]
fn test_predicate_decl_no_params() {
    // predicate AlwaysTrue() -> Bool { true }
    let span = test_span();

    let predicate = PredicateDecl {
        visibility: Visibility::Public,
        name: test_ident("AlwaysTrue"),
        generics: List::new(),
        params: List::new(),
        return_type: Type::bool(span),
        body: Heap::new(Expr::literal(Literal::bool(true, span))),
        span,
    };

    assert!(predicate.params.is_empty());
    assert_eq!(predicate.name.name.as_str(), "AlwaysTrue");
}

#[test]
fn test_predicate_decl_logical_or() {
    // predicate IsZeroOrOne(x: Int) -> Bool { x == 0 || x == 1 }
    let span = test_span();

    let left_cond = Expr::new(
        ExprKind::Binary {
            left: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("x"))),
                span,
            )),
            op: BinOp::Eq,
            right: Heap::new(Expr::literal(Literal::int(0, span))),
        },
        span,
    );

    let right_cond = Expr::new(
        ExprKind::Binary {
            left: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("x"))),
                span,
            )),
            op: BinOp::Eq,
            right: Heap::new(Expr::literal(Literal::int(1, span))),
        },
        span,
    );

    let body = Expr::new(
        ExprKind::Binary {
            left: Heap::new(left_cond),
            op: BinOp::Or,
            right: Heap::new(right_cond),
        },
        span,
    );

    let predicate = PredicateDecl {
        visibility: Visibility::Public,
        name: test_ident("IsZeroOrOne"),
        generics: List::new(),
        params: List::from(vec![FunctionParam::new(
            FunctionParamKind::Regular {
                pattern: Pattern::ident(test_ident("x"), false, span),
                ty: Type::int(span),
                default_value: Maybe::None,
            },
            span,
        )]),
        return_type: Type::bool(span),
        body: Heap::new(body),
        span,
    };

    match &predicate.body.kind {
        ExprKind::Binary { op, .. } => {
            assert_eq!(*op, BinOp::Or);
        }
        _ => panic!("Expected binary expression"),
    }
}

#[test]
fn test_predicate_decl_negation() {
    // predicate IsNegative(x: Int) -> Bool { !(x >= 0) }
    let span = test_span();

    let inner = Expr::new(
        ExprKind::Binary {
            left: Heap::new(Expr::new(
                ExprKind::Path(Path::single(test_ident("x"))),
                span,
            )),
            op: BinOp::Ge,
            right: Heap::new(Expr::literal(Literal::int(0, span))),
        },
        span,
    );

    let body = Expr::new(
        ExprKind::Unary {
            op: UnOp::Not,
            expr: Heap::new(inner),
        },
        span,
    );

    let predicate = PredicateDecl {
        visibility: Visibility::Public,
        name: test_ident("IsNegative"),
        generics: List::new(),
        params: List::from(vec![FunctionParam::new(
            FunctionParamKind::Regular {
                pattern: Pattern::ident(test_ident("x"), false, span),
                ty: Type::int(span),
                default_value: Maybe::None,
            },
            span,
        )]),
        return_type: Type::bool(span),
        body: Heap::new(body),
        span,
    };

    assert!(matches!(predicate.body.kind, ExprKind::Unary { .. }));
}
