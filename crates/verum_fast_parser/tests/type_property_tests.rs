//! Ambiguous names keep value syntax until declaration-aware resolution.
use verum_ast::{
    Expr, ExprKind, ItemKind, PatternKind, TypeKind, decl::FunctionParamKind, expr::TypeProperty,
};
use verum_fast_parser::Parser;

fn parse_expr(source: &str) -> Expr {
    Parser::new(source)
        .parse_expr()
        .unwrap_or_else(|error| panic!("{source}: {error:?}"))
}

fn named_field(expression: &Expr, name: &str, member: &str) {
    let ExprKind::Field { expr, field } = &expression.kind else {
        panic!("field: {expression:?}");
    };
    let ExprKind::Path(path) = &expr.kind else {
        panic!("path receiver: {expr:?}");
    };
    assert_eq!(path.to_string(), name);
    assert_eq!(field.name.as_str(), member);
}

#[test]
fn named_properties_preserve_receiver_identity_regardless_of_case() {
    for (name, member) in [
        ("Int", "size"),
        ("Float", "alignment"),
        ("Bool", "stride"),
        ("Int", "min"),
        ("Int", "max"),
        ("Int", "bits"),
        ("Text", "name"),
        ("Char", "size"),
        ("MyType", "size"),
        ("value", "size"),
        ("Value", "size"),
        ("x", "length"),
    ] {
        named_field(&parse_expr(&format!("{name}.{member}")), name, member);
    }
}

#[test]
fn qualified_properties_preserve_each_path_segment() {
    for root in ["std", "Std"] {
        let expr = parse_expr(&format!("{root}.collections.List.alignment"));
        let ExprKind::Field { expr, field } = expr.kind else {
            panic!("field");
        };
        assert_eq!(field.name.as_str(), "alignment");
        let ExprKind::Field { expr, field } = expr.kind else {
            panic!("List");
        };
        assert_eq!(field.name.as_str(), "List");
        named_field(&expr, root, "collections");
    }
}

#[test]
fn field_binding_survives_parentheses_references_and_expression_contexts() {
    for source in [
        "(Cell).size",
        "(&Cell).size",
        "(&mut Cell).size",
        "(&checked Cell).size",
        "(&unsafe Cell).size",
    ] {
        assert!(
            matches!(parse_expr(source).kind, ExprKind::Field { .. }),
            "{source}"
        );
    }
    let ExprKind::Binary { op, left, .. } = parse_expr("Int.size == 8").kind else {
        panic!("comparison");
    };
    assert_eq!(op, verum_ast::BinOp::Eq);
    named_field(&left, "Int", "size");
    let ExprKind::Call { args, .. } = parse_expr("allocate(MyStruct.size)").kind else {
        panic!("call");
    };
    assert_eq!(args.len(), 1);
    named_field(&args[0], "MyStruct", "size");
}

#[test]
fn explicit_unit_and_generic_type_properties_remain_structural() {
    let ExprKind::TypeProperty { ty, property } = parse_expr("().size").kind else {
        panic!("unit property");
    };
    assert!(matches!(ty.kind, TypeKind::Unit));
    assert_eq!(property, TypeProperty::Size);
    // Generic type expression syntax has already established the type shape.
    assert!(matches!(
        parse_expr("List<Int>.alignment").kind,
        ExprKind::TypeProperty { .. }
    ));
}

#[test]
fn bare_parameter_names_bind_while_structured_patterns_keep_their_shape() {
    for name in ["cell", "Cell", "Int"] {
        let ast = Parser::new(&format!("fn probe({name}: Cell) {{}}"))
            .parse_module()
            .unwrap();
        let ItemKind::Function(function) = &ast.items[0].kind else {
            panic!("function");
        };
        let FunctionParamKind::Regular { pattern, .. } = &function.params[0].kind else {
            panic!("parameter");
        };
        let PatternKind::Ident { name: parsed, .. } = &pattern.kind else {
            panic!("binding: {pattern:?}");
        };
        assert_eq!(parsed.name.as_str(), name);
    }
    for (prefix, by_ref, mutable) in [
        ("mut", false, true),
        ("ref", true, false),
        ("ref mut", true, true),
    ] {
        let ast = Parser::new(&format!("fn probe({prefix} Cell: Cell) {{}}"))
            .parse_module()
            .unwrap();
        let ItemKind::Function(function) = &ast.items[0].kind else {
            panic!("function");
        };
        let FunctionParamKind::Regular { pattern, .. } = &function.params[0].kind else {
            panic!("parameter");
        };
        let PatternKind::Ident {
            by_ref: actual_ref,
            mutable: actual_mut,
            name,
            ..
        } = &pattern.kind
        else {
            panic!("explicit binding: {pattern:?}");
        };
        assert_eq!(
            (name.name.as_str(), *actual_ref, *actual_mut),
            ("Cell", by_ref, mutable)
        );
    }
    for (source, expected) in [
        ("fn probe(UserId(n): UserId) {}", "variant"),
        ("fn probe((x, y): (Int, Int)) {}", "tuple"),
        ("fn probe(Choice.None: Choice) {}", "variant"),
        ("fn probe(Cell { size }: Cell) {}", "record"),
    ] {
        let ast = Parser::new(source).parse_module().unwrap();
        let ItemKind::Function(function) = &ast.items[0].kind else {
            panic!("function");
        };
        let FunctionParamKind::Regular { pattern, .. } = &function.params[0].kind else {
            panic!("parameter");
        };
        let actual = match pattern.kind {
            PatternKind::Variant { .. } => "variant",
            PatternKind::Tuple(_) => "tuple",
            PatternKind::Record { .. } => "record",
            _ => "other",
        };
        assert_eq!(actual, expected, "{source}");
    }
}
