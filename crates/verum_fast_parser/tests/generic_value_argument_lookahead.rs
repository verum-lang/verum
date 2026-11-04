//! Delimiter recognition must not reinterpret legal value arguments as comparisons.
use verum_ast::{ExprKind, expr::BinOp};
use verum_fast_parser::Parser;

#[test]
fn function_and_method_calls_preserve_scalar_value_arguments() {
    for argument in ["-1", "1/0", "5%2", "true", "false", "1.0", "\"1\"", "'x'"] {
        for source in [
            format!("apply<{argument}>(value)"),
            format!("owner.apply<{argument}>(value)"),
        ] {
            let expression = Parser::new(&source)
                .parse_expr()
                .unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
            let arguments = match expression.kind {
                ExprKind::Call { type_args, .. } | ExprKind::MethodCall { type_args, .. } => {
                    type_args
                }
                other => panic!("{source} became {other:?}"),
            };
            assert_eq!(arguments.len(), 1, "{source}");
            assert!(
                matches!(arguments[0], verum_ast::GenericArg::Const(_)),
                "{source}: {arguments:?}"
            );
        }
    }
}

#[test]
fn ordinary_comparisons_keep_their_expression_grammar() {
    for (source, expected) in [
        ("left < right", BinOp::Lt),
        ("left > right", BinOp::Gt),
        ("owner.field < 3", BinOp::Lt),
    ] {
        let expression = Parser::new(source).parse_expr().unwrap();
        assert!(
            matches!(expression.kind, ExprKind::Binary { op, .. } if op == expected),
            "{source}"
        );
    }
}

#[test]
fn nested_types_and_array_expressions_keep_their_delimiters() {
    for source in [
        "apply<List<Int>>(value)",
        "owner.apply<fn(Int)->Int>(value)",
        "apply<[Int; 1+1]>(value)",
    ] {
        let expression = Parser::new(source).parse_expr().unwrap();
        assert!(
            matches!(
                expression.kind,
                ExprKind::Call { .. } | ExprKind::MethodCall { .. }
            ),
            "{source}"
        );
    }
}

#[test]
fn static_nominal_value_arguments_use_the_same_scalar_grammar() {
    let expression = Parser::new("Cell<-1>.create()").parse_expr().unwrap();
    let ExprKind::MethodCall { receiver, .. } = expression.kind else {
        panic!("static method")
    };
    assert!(matches!(receiver.kind, ExprKind::TypeExpr(_)));
}
