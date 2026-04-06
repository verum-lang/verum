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
// Comprehensive tests for expression AST nodes
//
// Tests cover all expression types including:
// - Binary and unary operations
// - Function and method calls
// - Control flow (if, match, loops)
// - Closures and async expressions
// - Stream comprehensions
// - Tensor literals
// - Map and set literals
// - Error handling (try, ?, try-recover)
//
// Expression AST tests.

use verum_ast::expr::*;
use verum_ast::pattern::*;
use verum_ast::*;
use verum_common::List;
use verum_common::{Heap, Maybe, Text};

/// Helper function to create a test span
fn test_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

/// Helper function to create a test identifier
fn test_ident(name: &str) -> Ident {
    Ident::new(name, test_span())
}

/// Helper function to create a test path from a single identifier
fn test_path(name: &str) -> Path {
    Path::single(test_ident(name))
}

// ============================================================================
// CORRECTNESS TESTS - Basic Expression Construction
// ============================================================================

#[test]
fn test_literal_expr_construction() {
    let span = test_span();
    let lit = Literal::int(42, span);
    let expr = Expr::literal(lit.clone());

    match expr.kind {
        ExprKind::Literal(ref l) => {
            assert_eq!(*l, lit);
        }
        _ => panic!("Expected Literal expression"),
    }
    assert_eq!(expr.span, span);
}

#[test]
fn test_path_expr_construction() {
    let path = test_path("x");
    let expr = Expr::path(path.clone());

    match expr.kind {
        ExprKind::Path(ref p) => {
            assert_eq!(*p, path);
        }
        _ => panic!("Expected Path expression"),
    }
}

#[test]
fn test_ident_expr_construction() {
    let ident = test_ident("x");
    let expr = Expr::ident(ident.clone());

    match expr.kind {
        ExprKind::Path(ref p) => {
            assert!(p.is_single());
            assert_eq!(p.as_ident().unwrap().name, ident.name);
        }
        _ => panic!("Expected Path expression"),
    }
}

// ============================================================================
// BINARY OPERATIONS TESTS
// ============================================================================

#[test]
fn test_binary_arithmetic_operations() {
    let span = test_span();
    let left = Heap::new(Expr::ident(test_ident("a")));
    let right = Heap::new(Expr::ident(test_ident("b")));

    let ops = vec![
        (BinOp::Add, "+"),
        (BinOp::Sub, "-"),
        (BinOp::Mul, "*"),
        (BinOp::Div, "/"),
        (BinOp::Rem, "%"),
        (BinOp::Pow, "**"),
    ];

    for (op, expected_str) in ops {
        let expr = Expr::new(
            ExprKind::Binary {
                op,
                left: left.clone(),
                right: right.clone(),
            },
            span,
        );

        match expr.kind {
            ExprKind::Binary { op: actual_op, .. } => {
                assert_eq!(actual_op, op);
                assert_eq!(actual_op.as_str(), expected_str);
            }
            _ => panic!("Expected Binary expression"),
        }
    }
}

#[test]
fn test_binary_comparison_operations() {
    let span = test_span();
    let left = Heap::new(Expr::ident(test_ident("a")));
    let right = Heap::new(Expr::ident(test_ident("b")));

    let ops = vec![
        BinOp::Eq,
        BinOp::Ne,
        BinOp::Lt,
        BinOp::Le,
        BinOp::Gt,
        BinOp::Ge,
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

        match expr.kind {
            ExprKind::Binary { op: actual_op, .. } => {
                assert!(actual_op.is_comparison());
            }
            _ => panic!("Expected Binary expression"),
        }
    }
}

#[test]
fn test_binary_logical_operations() {
    let span = test_span();
    let left = Heap::new(Expr::ident(test_ident("a")));
    let right = Heap::new(Expr::ident(test_ident("b")));

    let ops = vec![BinOp::And, BinOp::Or, BinOp::Imply];

    for op in ops {
        let expr = Expr::new(
            ExprKind::Binary {
                op,
                left: left.clone(),
                right: right.clone(),
            },
            span,
        );

        match expr.kind {
            ExprKind::Binary { op: actual_op, .. } => {
                assert_eq!(actual_op, op);
            }
            _ => panic!("Expected Binary expression"),
        }
    }
}

#[test]
fn test_binary_bitwise_operations() {
    let span = test_span();
    let left = Heap::new(Expr::ident(test_ident("a")));
    let right = Heap::new(Expr::ident(test_ident("b")));

    let ops = vec![
        BinOp::BitAnd,
        BinOp::BitOr,
        BinOp::BitXor,
        BinOp::Shl,
        BinOp::Shr,
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

        match expr.kind {
            ExprKind::Binary { op: actual_op, .. } => {
                assert_eq!(actual_op, op);
            }
            _ => panic!("Expected Binary expression"),
        }
    }
}

#[test]
fn test_binary_assignment_operations() {
    let span = test_span();
    let left = Heap::new(Expr::ident(test_ident("a")));
    let right = Heap::new(Expr::ident(test_ident("b")));

    let ops = vec![
        BinOp::Assign,
        BinOp::AddAssign,
        BinOp::SubAssign,
        BinOp::MulAssign,
        BinOp::DivAssign,
        BinOp::RemAssign,
        BinOp::BitAndAssign,
        BinOp::BitOrAssign,
        BinOp::BitXorAssign,
        BinOp::ShlAssign,
        BinOp::ShrAssign,
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

        match expr.kind {
            ExprKind::Binary { op: actual_op, .. } => {
                assert!(actual_op.is_assignment());
            }
            _ => panic!("Expected Binary expression"),
        }
    }
}

#[test]
fn test_binop_is_commutative() {
    assert!(BinOp::Add.is_commutative());
    assert!(BinOp::Mul.is_commutative());
    assert!(BinOp::Eq.is_commutative());
    assert!(BinOp::Ne.is_commutative());
    assert!(BinOp::And.is_commutative());
    assert!(BinOp::Or.is_commutative());
    assert!(BinOp::BitAnd.is_commutative());
    assert!(BinOp::BitOr.is_commutative());
    assert!(BinOp::BitXor.is_commutative());

    assert!(!BinOp::Sub.is_commutative());
    assert!(!BinOp::Div.is_commutative());
    assert!(!BinOp::Lt.is_commutative());
    assert!(!BinOp::Shl.is_commutative());
}

// ============================================================================
// UNARY OPERATIONS TESTS
// ============================================================================

#[test]
fn test_unary_operations() {
    let span = test_span();
    let inner = Heap::new(Expr::ident(test_ident("x")));

    let ops = vec![
        (UnOp::Not, "!"),
        (UnOp::Neg, "-"),
        (UnOp::BitNot, "~"),
        (UnOp::Deref, "*"),
        (UnOp::Ref, "&"),
        (UnOp::RefMut, "&mut"),
        (UnOp::Own, "%"),
        (UnOp::OwnMut, "%mut"),
    ];

    for (op, expected_str) in ops {
        let expr = Expr::new(
            ExprKind::Unary {
                op,
                expr: inner.clone(),
            },
            span,
        );

        match expr.kind {
            ExprKind::Unary { op: actual_op, .. } => {
                assert_eq!(actual_op, op);
                assert_eq!(actual_op.as_str(), expected_str);
            }
            _ => panic!("Expected Unary expression"),
        }
    }
}

// ============================================================================
// CALL EXPRESSIONS TESTS
// ============================================================================

#[test]
fn test_function_call_no_args() {
    let span = test_span();
    let func = Heap::new(Expr::ident(test_ident("foo")));
    let args = List::new();

    let expr = Expr::new(
        ExprKind::Call {
            func,
            args: args.clone(),
            type_args: List::new(),
        },
        span,
    );

    match expr.kind {
        ExprKind::Call { ref args, .. } => {
            assert_eq!(args.len(), 0);
        }
        _ => panic!("Expected Call expression"),
    }
}

#[test]
fn test_function_call_with_args() {
    let span = test_span();
    let func = Heap::new(Expr::ident(test_ident("foo")));

    let mut args = List::new();
    args.push(Expr::literal(Literal::int(1, span)));
    args.push(Expr::literal(Literal::int(2, span)));
    args.push(Expr::literal(Literal::int(3, span)));

    let expr = Expr::new(
        ExprKind::Call {
            func,
            args: args.clone(),
            type_args: List::new(),
        },
        span,
    );

    match expr.kind {
        ExprKind::Call { ref args, .. } => {
            assert_eq!(args.len(), 3);
        }
        _ => panic!("Expected Call expression"),
    }
}

#[test]
fn test_method_call() {
    let span = test_span();
    let receiver = Heap::new(Expr::ident(test_ident("obj")));
    let method = test_ident("method");

    let mut args = List::new();
    args.push(Expr::literal(Literal::int(42, span)));

    let expr = Expr::new(
        ExprKind::MethodCall {
            receiver,
            method: method.clone(),
            args: args.clone(),
            type_args: List::new(),
        },
        span,
    );

    match expr.kind {
        ExprKind::MethodCall {
            ref method,
            ref args,
            ..
        } => {
            assert_eq!(method.name.as_str(), "method");
            assert_eq!(args.len(), 1);
        }
        _ => panic!("Expected MethodCall expression"),
    }
}

// ============================================================================
// FIELD ACCESS TESTS
// ============================================================================

#[test]
fn test_field_access() {
    let span = test_span();
    let expr = Heap::new(Expr::ident(test_ident("obj")));
    let field = test_ident("field");

    let field_expr = Expr::new(
        ExprKind::Field {
            expr,
            field: field.clone(),
        },
        span,
    );

    match field_expr.kind {
        ExprKind::Field { ref field, .. } => {
            assert_eq!(field.name.as_str(), "field");
        }
        _ => panic!("Expected Field expression"),
    }
}

#[test]
fn test_optional_chain() {
    let span = test_span();
    let expr = Heap::new(Expr::ident(test_ident("obj")));
    let field = test_ident("field");

    let chain_expr = Expr::new(
        ExprKind::OptionalChain {
            expr,
            field: field.clone(),
        },
        span,
    );

    match chain_expr.kind {
        ExprKind::OptionalChain { ref field, .. } => {
            assert_eq!(field.name.as_str(), "field");
        }
        _ => panic!("Expected OptionalChain expression"),
    }
}

#[test]
fn test_tuple_index() {
    let span = test_span();
    let expr = Heap::new(Expr::ident(test_ident("tuple")));

    let index_expr = Expr::new(ExprKind::TupleIndex { expr, index: 0 }, span);

    match index_expr.kind {
        ExprKind::TupleIndex { index, .. } => {
            assert_eq!(index, 0);
        }
        _ => panic!("Expected TupleIndex expression"),
    }
}

#[test]
fn test_index_operation() {
    let span = test_span();
    let expr = Heap::new(Expr::ident(test_ident("arr")));
    let index = Heap::new(Expr::literal(Literal::int(0, span)));

    let index_expr = Expr::new(ExprKind::Index { expr, index }, span);

    match index_expr.kind {
        ExprKind::Index { .. } => {}
        _ => panic!("Expected Index expression"),
    }
}

// ============================================================================
// PIPELINE AND NULL COALESCING TESTS
// ============================================================================

#[test]
fn test_pipeline_operator() {
    let span = test_span();
    let left = Heap::new(Expr::ident(test_ident("x")));
    let right = Heap::new(Expr::ident(test_ident("f")));

    let expr = Expr::new(ExprKind::Pipeline { left, right }, span);

    match expr.kind {
        ExprKind::Pipeline { .. } => {}
        _ => panic!("Expected Pipeline expression"),
    }
}

#[test]
fn test_null_coalesce() {
    let span = test_span();
    let left = Heap::new(Expr::ident(test_ident("a")));
    let right = Heap::new(Expr::ident(test_ident("b")));

    let expr = Expr::new(ExprKind::NullCoalesce { left, right }, span);

    match expr.kind {
        ExprKind::NullCoalesce { .. } => {}
        _ => panic!("Expected NullCoalesce expression"),
    }
}

// ============================================================================
// TYPE CAST AND ERROR PROPAGATION TESTS
// ============================================================================

#[test]
fn test_type_cast() {
    let span = test_span();
    let expr = Heap::new(Expr::ident(test_ident("x")));
    let ty = Type::int(span);

    let cast_expr = Expr::new(
        ExprKind::Cast {
            expr,
            ty: ty.clone(),
        },
        span,
    );

    match cast_expr.kind {
        ExprKind::Cast { ref ty, .. } => {
            assert!(matches!(ty.kind, TypeKind::Int));
        }
        _ => panic!("Expected Cast expression"),
    }
}

#[test]
fn test_try_operator() {
    let span = test_span();
    let inner = Heap::new(Expr::ident(test_ident("result")));

    let try_expr = Expr::new(ExprKind::Try(inner), span);

    match try_expr.kind {
        ExprKind::Try(_) => {}
        _ => panic!("Expected Try expression"),
    }
}

#[test]
fn test_try_recover() {
    let span = test_span();
    let try_block = Heap::new(Expr::new(ExprKind::Block(Block::empty(span)), span));
    let recover = RecoverBody::MatchArms {
        arms: List::new(),
        span,
    };

    let expr = Expr::new(
        ExprKind::TryRecover {
            try_block,
            recover,
        },
        span,
    );

    match expr.kind {
        ExprKind::TryRecover { .. } => {}
        _ => panic!("Expected TryRecover expression"),
    }
}

#[test]
fn test_try_finally() {
    let span = test_span();
    let try_block = Heap::new(Expr::new(ExprKind::Block(Block::empty(span)), span));
    let finally_block = Heap::new(Expr::new(ExprKind::Block(Block::empty(span)), span));

    let expr = Expr::new(
        ExprKind::TryFinally {
            try_block,
            finally_block,
        },
        span,
    );

    match expr.kind {
        ExprKind::TryFinally { .. } => {}
        _ => panic!("Expected TryFinally expression"),
    }
}

#[test]
fn test_try_recover_finally() {
    let span = test_span();
    let try_block = Heap::new(Expr::new(ExprKind::Block(Block::empty(span)), span));
    let recover = RecoverBody::MatchArms {
        arms: List::new(),
        span,
    };
    let finally_block = Heap::new(Expr::new(ExprKind::Block(Block::empty(span)), span));

    let expr = Expr::new(
        ExprKind::TryRecoverFinally {
            try_block,
            recover,
            finally_block,
        },
        span,
    );

    match expr.kind {
        ExprKind::TryRecoverFinally { .. } => {}
        _ => panic!("Expected TryRecoverFinally expression"),
    }
}

// ============================================================================
// COLLECTION LITERALS TESTS
// ============================================================================

#[test]
fn test_tuple_expression() {
    let span = test_span();
    let mut elements = List::new();
    elements.push(Expr::literal(Literal::int(1, span)));
    elements.push(Expr::literal(Literal::int(2, span)));
    elements.push(Expr::literal(Literal::int(3, span)));

    let expr = Expr::new(ExprKind::Tuple(elements.clone()), span);

    match expr.kind {
        ExprKind::Tuple(ref elems) => {
            assert_eq!(elems.len(), 3);
        }
        _ => panic!("Expected Tuple expression"),
    }
}

#[test]
fn test_array_list() {
    let span = test_span();
    let mut elements = List::new();
    elements.push(Expr::literal(Literal::int(1, span)));
    elements.push(Expr::literal(Literal::int(2, span)));

    let expr = Expr::new(ExprKind::Array(ArrayExpr::List(elements.clone())), span);

    match expr.kind {
        ExprKind::Array(ArrayExpr::List(ref elems)) => {
            assert_eq!(elems.len(), 2);
        }
        _ => panic!("Expected Array List expression"),
    }
}

#[test]
fn test_array_repeat() {
    let span = test_span();
    let value = Heap::new(Expr::literal(Literal::int(0, span)));
    let count = Heap::new(Expr::literal(Literal::int(10, span)));

    let expr = Expr::new(ExprKind::Array(ArrayExpr::Repeat { value, count }), span);

    match expr.kind {
        ExprKind::Array(ArrayExpr::Repeat { .. }) => {}
        _ => panic!("Expected Array Repeat expression"),
    }
}

#[test]
fn test_map_literal() {
    let span = test_span();
    let mut entries = List::new();
    entries.push((
        Expr::literal(Literal::string("key".to_string().into(), span)),
        Expr::literal(Literal::int(42, span)),
    ));

    let expr = Expr::new(
        ExprKind::MapLiteral {
            entries: entries.clone(),
        },
        span,
    );

    match expr.kind {
        ExprKind::MapLiteral { ref entries } => {
            assert_eq!(entries.len(), 1);
        }
        _ => panic!("Expected MapLiteral expression"),
    }
}

#[test]
fn test_set_literal() {
    let span = test_span();
    let mut elements = List::new();
    elements.push(Expr::literal(Literal::int(1, span)));
    elements.push(Expr::literal(Literal::int(2, span)));

    let expr = Expr::new(
        ExprKind::SetLiteral {
            elements: elements.clone(),
        },
        span,
    );

    match expr.kind {
        ExprKind::SetLiteral { ref elements } => {
            assert_eq!(elements.len(), 2);
        }
        _ => panic!("Expected SetLiteral expression"),
    }
}

// ============================================================================
// COMPREHENSION TESTS
// ============================================================================

#[test]
fn test_list_comprehension() {
    let span = test_span();
    let expr = Heap::new(Expr::ident(test_ident("x")));

    let mut clauses = List::new();
    clauses.push(ComprehensionClause {
        kind: ComprehensionClauseKind::For {
            pattern: Pattern::ident(test_ident("x"), false, span),
            iter: Expr::ident(test_ident("list")),
        },
        span,
    });

    let comp = Expr::new(ExprKind::Comprehension { expr, clauses }, span);

    match comp.kind {
        ExprKind::Comprehension { ref clauses, .. } => {
            assert_eq!(clauses.len(), 1);
        }
        _ => panic!("Expected Comprehension expression"),
    }
}

#[test]
fn test_stream_comprehension() {
    let span = test_span();
    let expr = Heap::new(Expr::ident(test_ident("x")));

    let mut clauses = List::new();
    clauses.push(ComprehensionClause {
        kind: ComprehensionClauseKind::For {
            pattern: Pattern::ident(test_ident("x"), false, span),
            iter: Expr::ident(test_ident("stream")),
        },
        span,
    });

    let comp = Expr::new(ExprKind::StreamComprehension { expr, clauses }, span);

    match comp.kind {
        ExprKind::StreamComprehension { ref clauses, .. } => {
            assert_eq!(clauses.len(), 1);
        }
        _ => panic!("Expected StreamComprehension expression"),
    }
}

#[test]
fn test_comprehension_with_if_clause() {
    let span = test_span();
    let expr = Heap::new(Expr::ident(test_ident("x")));

    let mut clauses = List::new();
    clauses.push(ComprehensionClause {
        kind: ComprehensionClauseKind::For {
            pattern: Pattern::ident(test_ident("x"), false, span),
            iter: Expr::ident(test_ident("list")),
        },
        span,
    });
    clauses.push(ComprehensionClause {
        kind: ComprehensionClauseKind::If(Expr::literal(Literal::bool(true, span))),
        span,
    });

    let comp = Expr::new(ExprKind::Comprehension { expr, clauses }, span);

    match comp.kind {
        ExprKind::Comprehension { ref clauses, .. } => {
            assert_eq!(clauses.len(), 2);
        }
        _ => panic!("Expected Comprehension expression"),
    }
}

// ============================================================================
// RECORD EXPRESSION TESTS
// ============================================================================

#[test]
fn test_record_expression() {
    let span = test_span();
    let path = test_path("Point");

    let mut fields = List::new();
    fields.push(FieldInit::new(
        test_ident("x"),
        Maybe::Some(Expr::literal(Literal::int(1, span))),
        span,
    ));
    fields.push(FieldInit::new(
        test_ident("y"),
        Maybe::Some(Expr::literal(Literal::int(2, span))),
        span,
    ));

    let expr = Expr::new(
        ExprKind::Record {
            path,
            fields: fields.clone(),
            base: Maybe::None,
        },
        span,
    );

    match expr.kind {
        ExprKind::Record { ref fields, .. } => {
            assert_eq!(fields.len(), 2);
        }
        _ => panic!("Expected Record expression"),
    }
}

#[test]
fn test_record_expression_shorthand() {
    let span = test_span();
    let path = test_path("Point");

    let mut fields = List::new();
    fields.push(FieldInit::shorthand(test_ident("x")));

    let expr = Expr::new(
        ExprKind::Record {
            path,
            fields: fields.clone(),
            base: Maybe::None,
        },
        span,
    );

    match expr.kind {
        ExprKind::Record { ref fields, .. } => {
            assert_eq!(fields.len(), 1);
            assert!(matches!(fields[0].value, Maybe::None));
        }
        _ => panic!("Expected Record expression"),
    }
}

#[test]
fn test_record_expression_with_base() {
    let span = test_span();
    let path = test_path("Point");
    let base = Maybe::Some(Heap::new(Expr::ident(test_ident("base"))));

    let mut fields = List::new();
    fields.push(FieldInit::new(
        test_ident("x"),
        Maybe::Some(Expr::literal(Literal::int(5, span))),
        span,
    ));

    let expr = Expr::new(
        ExprKind::Record {
            path,
            fields,
            base: base.clone(),
        },
        span,
    );

    match expr.kind {
        ExprKind::Record { ref base, .. } => {
            assert!(matches!(base, Maybe::Some(_)));
        }
        _ => panic!("Expected Record expression"),
    }
}

// ============================================================================
// TENSOR LITERAL TESTS - compile-time shape validation
// ============================================================================

#[test]
fn test_tensor_literal_1d() {
    let span = test_span();
    let mut shape = List::new();
    shape.push(4);

    let elem_type = Type::int(span);
    let data = Heap::new(Expr::ident(test_ident("data")));

    let expr = Expr::new(
        ExprKind::TensorLiteral {
            shape: shape.clone(),
            elem_type,
            data,
        },
        span,
    );

    match expr.kind {
        ExprKind::TensorLiteral { ref shape, .. } => {
            assert_eq!(shape.len(), 1);
            assert_eq!(shape[0], 4);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

#[test]
fn test_tensor_literal_2d() {
    let span = test_span();
    let mut shape = List::new();
    shape.push(2);
    shape.push(3);

    let elem_type = Type::int(span);
    let data = Heap::new(Expr::ident(test_ident("data")));

    let expr = Expr::new(
        ExprKind::TensorLiteral {
            shape: shape.clone(),
            elem_type,
            data,
        },
        span,
    );

    match expr.kind {
        ExprKind::TensorLiteral { ref shape, .. } => {
            assert_eq!(shape.len(), 2);
            assert_eq!(shape[0], 2);
            assert_eq!(shape[1], 3);
        }
        _ => panic!("Expected TensorLiteral expression"),
    }
}

// ============================================================================
// INTERPOLATED STRING TESTS
// ============================================================================

#[test]
fn test_interpolated_string_expression() {
    let span = test_span();
    let handler = Text::from("f");

    let mut parts = List::new();
    parts.push(Text::from("Hello "));
    parts.push(Text::from("!"));

    let mut exprs = List::new();
    exprs.push(Expr::ident(test_ident("name")));

    let expr = Expr::new(
        ExprKind::InterpolatedString {
            handler: handler.clone(),
            parts,
            exprs,
        },
        span,
    );

    match expr.kind {
        ExprKind::InterpolatedString {
            ref handler,
            ref parts,
            ref exprs,
        } => {
            assert_eq!(handler.as_str(), "f");
            assert_eq!(parts.len(), 2);
            assert_eq!(exprs.len(), 1);
        }
        _ => panic!("Expected InterpolatedString expression"),
    }
}

// ============================================================================
// CONTROL FLOW TESTS
// ============================================================================

#[test]
fn test_if_expression() {
    let span = test_span();
    let condition = Heap::new(IfCondition {
        conditions: smallvec::smallvec![ConditionKind::Expr(Expr::literal(Literal::bool(
            true, span
        )))],
        span,
    });
    let then_branch = Block::empty(span);
    let else_branch = Maybe::None;

    let expr = Expr::new(
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        },
        span,
    );

    match expr.kind {
        ExprKind::If { .. } => {}
        _ => panic!("Expected If expression"),
    }
}

#[test]
fn test_if_let_expression() {
    let span = test_span();
    let condition = Heap::new(IfCondition {
        conditions: smallvec::smallvec![ConditionKind::Let {
            pattern: Pattern::ident(test_ident("x"), false, span),
            value: Expr::ident(test_ident("opt")),
        }],
        span,
    });
    let then_branch = Block::empty(span);

    let expr = Expr::new(
        ExprKind::If {
            condition,
            then_branch,
            else_branch: Maybe::None,
        },
        span,
    );

    match expr.kind {
        ExprKind::If { .. } => {}
        _ => panic!("Expected If expression"),
    }
}

#[test]
fn test_match_expression() {
    let span = test_span();
    let expr = Heap::new(Expr::ident(test_ident("value")));

    let mut arms = List::new();
    arms.push(MatchArm::new(
        Pattern::wildcard(span),
        Maybe::None,
        Heap::new(Expr::literal(Literal::int(0, span))),
        span,
    ));

    let match_expr = Expr::new(ExprKind::Match { expr, arms }, span);

    match match_expr.kind {
        ExprKind::Match { ref arms, .. } => {
            assert_eq!(arms.len(), 1);
        }
        _ => panic!("Expected Match expression"),
    }
}

#[test]
fn test_loop_expression() {
    let span = test_span();
    let body = Block::empty(span);

    let expr = Expr::new(
        ExprKind::Loop {
            label: Maybe::None,
            body,
            invariants: List::new(),
        },
        span,
    );

    match expr.kind {
        ExprKind::Loop { .. } => {}
        _ => panic!("Expected Loop expression"),
    }
}

#[test]
fn test_while_expression() {
    let span = test_span();
    let condition = Heap::new(Expr::literal(Literal::bool(true, span)));
    let body = Block::empty(span);

    let expr = Expr::new(
        ExprKind::While {
            label: Maybe::None,
            condition,
            body,
            invariants: List::new(),
            decreases: List::new(),
        },
        span,
    );

    match expr.kind {
        ExprKind::While { .. } => {}
        _ => panic!("Expected While expression"),
    }
}

#[test]
fn test_for_expression() {
    let span = test_span();
    let pattern = Pattern::ident(test_ident("x"), false, span);
    let iter = Heap::new(Expr::ident(test_ident("list")));
    let body = Block::empty(span);

    let expr = Expr::new(
        ExprKind::For {
            label: Maybe::None,
            pattern,
            iter,
            body,
            invariants: List::new(),
            decreases: List::new(),
        },
        span,
    );

    match expr.kind {
        ExprKind::For { .. } => {}
        _ => panic!("Expected For expression"),
    }
}

#[test]
fn test_break_expression() {
    let span = test_span();
    let expr = Expr::new(
        ExprKind::Break {
            label: Maybe::None,
            value: Maybe::None,
        },
        span,
    );

    match expr.kind {
        ExprKind::Break {
            label: Maybe::None,
            value: Maybe::None,
        } => {}
        _ => panic!("Expected Break expression"),
    }
}

#[test]
fn test_break_with_value() {
    let span = test_span();
    let value = Maybe::Some(Heap::new(Expr::literal(Literal::int(42, span))));
    let expr = Expr::new(
        ExprKind::Break {
            label: Maybe::None,
            value,
        },
        span,
    );

    match expr.kind {
        ExprKind::Break {
            value: Maybe::Some(_),
            ..
        } => {}
        _ => panic!("Expected Break expression with value"),
    }
}

#[test]
fn test_continue_expression() {
    let span = test_span();
    let expr = Expr::new(ExprKind::Continue { label: Maybe::None }, span);

    match expr.kind {
        ExprKind::Continue { .. } => {}
        _ => panic!("Expected Continue expression"),
    }
}

#[test]
fn test_return_expression() {
    let span = test_span();
    let expr = Expr::new(ExprKind::Return(Maybe::None), span);

    match expr.kind {
        ExprKind::Return(Maybe::None) => {}
        _ => panic!("Expected Return expression"),
    }
}

#[test]
fn test_return_with_value() {
    let span = test_span();
    let value = Maybe::Some(Heap::new(Expr::literal(Literal::int(42, span))));
    let expr = Expr::new(ExprKind::Return(value), span);

    match expr.kind {
        ExprKind::Return(Maybe::Some(_)) => {}
        _ => panic!("Expected Return expression with value"),
    }
}

#[test]
fn test_yield_expression() {
    let span = test_span();
    let value = Heap::new(Expr::literal(Literal::int(42, span)));
    let expr = Expr::new(ExprKind::Yield(value), span);

    match expr.kind {
        ExprKind::Yield(_) => {}
        _ => panic!("Expected Yield expression"),
    }
}

// ============================================================================
// CLOSURE AND ASYNC TESTS
// ============================================================================

#[test]
fn test_closure_expression() {
    let span = test_span();
    let params = List::new();
    let return_type = Maybe::None;
    let body = Heap::new(Expr::literal(Literal::int(42, span)));

    let expr = Expr::new(
        ExprKind::Closure {
            async_: false,
            move_: false,
            params,
            contexts: List::new(),
            return_type,
            body,
        },
        span,
    );

    match expr.kind {
        ExprKind::Closure { async_, .. } => {
            assert!(!async_);
        }
        _ => panic!("Expected Closure expression"),
    }
}

#[test]
fn test_async_closure() {
    let span = test_span();
    let params = List::new();
    let return_type = Maybe::None;
    let body = Heap::new(Expr::literal(Literal::int(42, span)));

    let expr = Expr::new(
        ExprKind::Closure {
            async_: true,
            move_: false,
            params,
            contexts: List::new(),
            return_type,
            body,
        },
        span,
    );

    match expr.kind {
        ExprKind::Closure { async_, .. } => {
            assert!(async_);
        }
        _ => panic!("Expected async Closure expression"),
    }
}

#[test]
fn test_closure_with_params() {
    let span = test_span();

    let mut params = List::new();
    params.push(ClosureParam::new(
        Pattern::ident(test_ident("x"), false, span),
        Maybe::None,
        span,
    ));

    let body = Heap::new(Expr::ident(test_ident("x")));

    let expr = Expr::new(
        ExprKind::Closure {
            async_: false,
            move_: false,
            params: params.clone(),
            contexts: List::new(),
            return_type: Maybe::None,
            body,
        },
        span,
    );

    match expr.kind {
        ExprKind::Closure { ref params, .. } => {
            assert_eq!(params.len(), 1);
        }
        _ => panic!("Expected Closure expression"),
    }
}

#[test]
fn test_async_block() {
    let span = test_span();
    let block = Block::empty(span);

    let expr = Expr::new(ExprKind::Async(block), span);

    match expr.kind {
        ExprKind::Async(_) => {}
        _ => panic!("Expected Async expression"),
    }
}

#[test]
fn test_await_expression() {
    let span = test_span();
    let future = Heap::new(Expr::ident(test_ident("future")));

    let expr = Expr::new(ExprKind::Await(future), span);

    match expr.kind {
        ExprKind::Await(_) => {}
        _ => panic!("Expected Await expression"),
    }
}

#[test]
fn test_spawn_expression() {
    let span = test_span();
    let expr_to_spawn = Heap::new(Expr::literal(Literal::int(42, span)));
    let contexts = List::new();

    let expr = Expr::new(
        ExprKind::Spawn {
            expr: expr_to_spawn,
            contexts,
        },
        span,
    );

    match expr.kind {
        ExprKind::Spawn { .. } => {}
        _ => panic!("Expected Spawn expression"),
    }
}

// ============================================================================
// SPECIAL BLOCKS TESTS
// ============================================================================

#[test]
fn test_unsafe_block() {
    let span = test_span();
    let block = Block::empty(span);

    let expr = Expr::new(ExprKind::Unsafe(block), span);

    match expr.kind {
        ExprKind::Unsafe(_) => {}
        _ => panic!("Expected Unsafe expression"),
    }
}

#[test]
fn test_meta_block() {
    let span = test_span();
    let block = Block::empty(span);

    let expr = Expr::new(ExprKind::Meta(block), span);

    match expr.kind {
        ExprKind::Meta(_) => {}
        _ => panic!("Expected Meta expression"),
    }
}

#[test]
fn test_use_context_expression() {
    let span = test_span();
    let context = test_path("State");
    let handler = Heap::new(Expr::ident(test_ident("handler")));
    let body = Heap::new(Expr::literal(Literal::int(42, span)));

    let expr = Expr::new(
        ExprKind::UseContext {
            context,
            handler,
            body,
        },
        span,
    );

    match expr.kind {
        ExprKind::UseContext { .. } => {}
        _ => panic!("Expected UseContext expression"),
    }
}

// ============================================================================
// RANGE AND PARENTHESIZED TESTS
// ============================================================================

#[test]
fn test_range_expression() {
    let span = test_span();
    let start = Maybe::Some(Heap::new(Expr::literal(Literal::int(0, span))));
    let end = Maybe::Some(Heap::new(Expr::literal(Literal::int(10, span))));

    let expr = Expr::new(
        ExprKind::Range {
            start,
            end,
            inclusive: false,
        },
        span,
    );

    match expr.kind {
        ExprKind::Range { inclusive, .. } => {
            assert!(!inclusive);
        }
        _ => panic!("Expected Range expression"),
    }
}

#[test]
fn test_range_inclusive() {
    let span = test_span();
    let start = Maybe::Some(Heap::new(Expr::literal(Literal::int(0, span))));
    let end = Maybe::Some(Heap::new(Expr::literal(Literal::int(10, span))));

    let expr = Expr::new(
        ExprKind::Range {
            start,
            end,
            inclusive: true,
        },
        span,
    );

    match expr.kind {
        ExprKind::Range { inclusive, .. } => {
            assert!(inclusive);
        }
        _ => panic!("Expected Range expression"),
    }
}

#[test]
fn test_parenthesized_expression() {
    let span = test_span();
    let inner = Heap::new(Expr::literal(Literal::int(42, span)));

    let expr = Expr::new(ExprKind::Paren(inner), span);

    match expr.kind {
        ExprKind::Paren(_) => {}
        _ => panic!("Expected Paren expression"),
    }
}

// ============================================================================
// BLOCK TESTS
// ============================================================================

#[test]
fn test_block_empty() {
    let span = test_span();
    let block = Block::empty(span);

    assert_eq!(block.stmts.len(), 0);
    assert!(matches!(block.expr, Maybe::None));
    assert_eq!(block.span, span);
}

#[test]
fn test_block_with_stmts() {
    let span = test_span();
    let mut stmts = List::new();
    stmts.push(Stmt::expr(Expr::literal(Literal::int(1, span)), true));

    let block = Block::new(stmts.clone(), Maybe::None, span);

    assert_eq!(block.stmts.len(), 1);
    assert!(matches!(block.expr, Maybe::None));
}

#[test]
fn test_block_with_expr() {
    let span = test_span();
    let stmts = List::new();
    let expr = Maybe::Some(Heap::new(Expr::literal(Literal::int(42, span))));

    let block = Block::new(stmts, expr.clone(), span);

    assert_eq!(block.stmts.len(), 0);
    assert!(matches!(block.expr, Maybe::Some(_)));
}

// ============================================================================
// SAFETY TESTS - No panics
// ============================================================================

#[test]
fn test_expr_construction_never_panics() {
    let span = test_span();

    // All constructors should work
    let _ = Expr::literal(Literal::int(42, span));
    let _ = Expr::path(test_path("x"));
    let _ = Expr::ident(test_ident("x"));
}

#[test]
fn test_deep_expression_tree() {
    let span = test_span();
    let mut expr = Expr::literal(Literal::int(0, span));

    // Build a deep tree
    for i in 1..100 {
        expr = Expr::new(
            ExprKind::Binary {
                op: BinOp::Add,
                left: Heap::new(expr),
                right: Heap::new(Expr::literal(Literal::int(i, span))),
            },
            span,
        );
    }

    // Should be able to create and work with deep trees
    fn count_nodes(expr: &Expr) -> usize {
        match &expr.kind {
            ExprKind::Binary { left, right, .. } => 1 + count_nodes(left) + count_nodes(right),
            _ => 1,
        }
    }

    assert!(count_nodes(&expr) > 100);
}
