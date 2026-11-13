//! Associated declarations keep type refinements and declaration constraints apart.
use verum_ast::{ItemKind, TypeDeclBody, TypeKind, decl::ProtocolItemKind};
use verum_fast_parser::Parser;

fn associated(source: &str) -> ProtocolItemKind {
    let module = Parser::new(source).parse_module().unwrap();
    let ItemKind::Type(declaration) = &module.items[0].kind else {
        panic!("protocol type");
    };
    let TypeDeclBody::Protocol(body) = &declaration.body else {
        panic!("protocol body");
    };
    body.items[0].kind.clone()
}

#[test]
fn inline_refinement_preserves_its_predicate_and_outer_where_clause() {
    for declaration in [
        "type Size where type Self: Sized = Int{it > 0};",
        "type Size = Int{it > 0} where type Self: Sized;",
    ] {
        let ProtocolItemKind::Type {
            default_type,
            where_clause,
            ..
        } = associated(&format!("type Container is protocol {{ {declaration} }};"))
        else {
            panic!("associated declaration");
        };
        let default_type = default_type.expect("default");
        let TypeKind::Refined { base, predicate } = default_type.kind else {
            panic!("inline refinement");
        };
        assert!(matches!(base.kind, TypeKind::Int));
        assert!(matches!(
            predicate.expr.kind,
            verum_ast::ExprKind::Binary { .. }
        ));
        assert_eq!(
            where_clause
                .expect("associated constraint")
                .predicates
                .len(),
            1
        );
    }
}

#[test]
fn associated_defaults_preserve_nested_function_and_sigma_types() {
    for (ty, function) in [
        ("fn(Int) -> Int{> 0}", true),
        ("fn(Int) -> n: Int where n > 0", true),
        ("n: Int where n > 0", false),
    ] {
        let ProtocolItemKind::Type { default_type, .. } =
            associated(&format!("protocol Container {{ type Item = {ty}; }}"))
        else {
            panic!("associated declaration");
        };
        let kind = default_type.expect("default").kind;
        if function {
            let TypeKind::Function { return_type, .. } = kind else {
                panic!("function type");
            };
            assert!(matches!(return_type.kind, TypeKind::Refined { .. }));
        } else {
            let TypeKind::Refined { predicate, .. } = kind else {
                panic!("sigma refinement");
            };
            assert_eq!(predicate.binding.as_ref().unwrap().name.as_str(), "n");
        }
    }
}

#[test]
fn duplicate_associated_where_clauses_are_refused() {
    assert!(
        Parser::new(
            "protocol Container { type Size where type Self: Sized = Int where type Self: Copy; }"
        )
        .parse_module()
        .is_err()
    );
}
