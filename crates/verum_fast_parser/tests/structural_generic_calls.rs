//! Explicit generic calls preserve structural types, not comparison expressions.
use verum_ast::{Expr, ExprKind, GenericArg, TypeKind, span::FileId};
use verum_common::Text;
use verum_fast_parser::RecursiveParser;
use verum_lexer::Lexer;

fn parse(source: &str) -> Expr {
    let file = FileId::new(0);
    let tokens = Lexer::new(source, file).tokenize().expect("lex expression");
    let mut parser = RecursiveParser::new(&tokens, file);
    let expr = parser.parse_expr().expect("parse expression");
    assert!(
        parser.stream.at_end(),
        "unconsumed input for {source}: {expr:?}"
    );
    expr
}

fn argument(source: &str) -> TypeKind {
    let expr = parse(source);
    let args = match expr.kind {
        ExprKind::Call { type_args, .. } | ExprKind::MethodCall { type_args, .. } => type_args,
        other => panic!("expected genuine call for {source}, got {other:?}"),
    };
    assert_eq!(args.len(), 1, "{source}");
    let GenericArg::Type(ty) = &args[0] else {
        panic!("expected type for {source}: {args:?}")
    };
    ty.kind.clone()
}

fn shape(ty: &TypeKind) -> Text {
    match ty {
        TypeKind::Path(path) => Text::from(path.last_segment_name()),
        TypeKind::Reference { mutable, inner } => Text::from(format!(
            "&{}{}",
            if *mutable { "mut " } else { "" },
            shape(&inner.kind)
        )),
        TypeKind::CheckedReference { mutable, inner } => Text::from(format!(
            "&checked {}{}",
            if *mutable { "mut " } else { "" },
            shape(&inner.kind)
        )),
        TypeKind::UnsafeReference { mutable, inner } => Text::from(format!(
            "&unsafe {}{}",
            if *mutable { "mut " } else { "" },
            shape(&inner.kind)
        )),
        TypeKind::Array {
            element,
            size: Some(size),
        } => {
            let ExprKind::Literal(literal) = &size.kind else {
                panic!("literal count")
            };
            let verum_ast::literal::LiteralKind::Int(count) = &literal.kind else {
                panic!("integer count")
            };
            Text::from(format!("[{}; {}]", shape(&element.kind), count.value))
        }
        TypeKind::Slice(element) => Text::from(format!("[{}]", shape(&element.kind))),
        other => panic!("unexpected structural type: {other:?}"),
    }
}

#[test]
fn explicit_reference_arguments_are_genuine_calls() {
    for site in ["size", "object.size", "util.size"] {
        for ty in [
            "&Byte",
            "&mut Byte",
            "&checked Byte",
            "&unsafe Byte",
            "&checked mut Byte",
            "&unsafe mut Byte",
            "&[Byte]",
        ] {
            let source = format!("{site}<{ty}>()");
            assert_eq!(shape(&argument(&source)).as_str(), ty, "{source}");
        }
    }
}

#[test]
fn named_and_literal_const_arrays_retain_their_existing_argument_kind() {
    for source in [
        "Shape<[N; M]>",
        "Shape<[N]>",
        "Shape<[3; 4]>",
        "Shape<[3, 4]>",
    ] {
        let ty = verum_fast_parser::Parser::new(source)
            .parse_type()
            .expect("generic type");
        let TypeKind::Generic { args, .. } = ty.kind else {
            panic!("generic type")
        };
        assert!(
            matches!(&args[0], GenericArg::Const(expr) if matches!(expr.kind, ExprKind::Array(_))),
            "{source}: {args:?}"
        );
    }
}

#[test]
fn malformed_structural_types_do_not_become_valid_calls() {
    for source in ["size<&mut>()", "size<&checked>()", "size<&unsafe mut>()"] {
        let file = FileId::new(0);
        let tokens = Lexer::new(source, file)
            .tokenize()
            .expect("lex malformed expression");
        let mut parser = RecursiveParser::new(&tokens, file);
        let result = parser.parse_expr();
        assert!(
            result.is_err() || !parser.stream.at_end(),
            "{source}: {result:?}"
        );
    }
}

#[test]
fn nominal_calls_and_comparisons_remain_distinct() {
    assert!(matches!(argument("size<Byte>()"), TypeKind::Path(_)));
    for source in [
        "left < right",
        "left < (right & mask)",
        "(left & mask) < right",
        "left < &right",
        "left < [right; 4]",
    ] {
        assert!(
            matches!(parse(source).kind, ExprKind::Binary { .. }),
            "{source}"
        );
    }
}

#[test]
fn reference_lifetime_syntax_reaches_the_type_parser() {
    assert_eq!(shape(&argument("size<&'a Byte>()")).as_str(), "&Byte");
    let ExprKind::Call { type_args, .. } = parse("choose<'a, &Byte>()").kind else {
        panic!("call")
    };
    assert!(matches!(&type_args[0], GenericArg::Lifetime(_)));
    assert!(
        matches!(&type_args[1], GenericArg::Type(ty) if matches!(ty.kind, TypeKind::Reference { .. }))
    );
}

#[test]
fn bracket_arguments_are_calls_but_keep_lossless_value_syntax() {
    for callee in ["size", "util.size", "object.size"] {
        for spelling in [
            "[Byte; 3]",
            "[[Byte; 2]; 3]",
            "[Byte]",
            "[N; M]",
            "[N]",
            "[3; 4]",
            "[3, 4]",
        ] {
            let source = format!("{callee}<{spelling}>()");
            let call = parse(&source);
            let arguments = match call.kind {
                ExprKind::Call { type_args, .. } | ExprKind::MethodCall { type_args, .. } => {
                    type_args
                }
                other => panic!("genuine parsed call required: {source}: {other:?}"),
            };
            assert!(
                matches!(&arguments[0], GenericArg::Const(expression) if matches!(expression.kind, ExprKind::Array(_))),
                "declaration must select the role: {source}: {arguments:?}"
            );
        }
    }
}

#[test]
fn semicolons_outside_brackets_do_not_prove_a_generic_call() {
    for source in ["size<Byte; 3>()", "size<[Byte; 3>()", "size<Byte]; 3>()"] {
        let file = FileId::new(0);
        let tokens = Lexer::new(source, file)
            .tokenize()
            .expect("lex malformed expression");
        let mut parser = RecursiveParser::new(&tokens, file);
        let result = parser.parse_expr();
        assert!(
            result.is_err() || !parser.stream.at_end(),
            "{source}: {result:?}"
        );
    }
}

#[test]
fn bracket_comparisons_do_not_close_the_outer_generic_argument_list() {
    for source in [
        "size<[Byte; if 2 > 1 { 3 } else { 4 }]>()",
        "object.size<[Byte; if 2 < 1 { 3 } else { 4 }]>()",
        "size<[Byte; 8 >> 1]>()",
    ] {
        let call = parse(source);
        let args = match call.kind {
            ExprKind::Call { type_args, .. } | ExprKind::MethodCall { type_args, .. } => type_args,
            other => panic!("call expected: {source}: {other:?}"),
        };
        assert!(
            matches!(&args[0], GenericArg::Const(expr) if matches!(expr.kind, ExprKind::Array(_)))
        );
    }
    for source in [
        "left < [right; if 2 > 1 {3} else {4}]",
        "left < [right; 8 >> 1]",
    ] {
        assert!(
            matches!(parse(source).kind, ExprKind::Binary { .. }),
            "{source}"
        );
    }
    assert!(matches!(
        argument("size<fn([Byte; 3]) -> Int>()"),
        TypeKind::Function { .. }
    ));
}
