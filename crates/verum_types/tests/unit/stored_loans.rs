use super::{Bindings, MAX_LOAN_SHAPE_STEPS, Node, Type, TypeChecker};
use verum_common::List;

#[test]
fn every_resolution_at_a_source_site_must_consume_the_receiver() {
    use crate::core_metadata::ReceiverKind::{SelfRef, SelfValue};
    use crate::type_registry::TypeRegistry;
    use verum_ast::{FileId, Span};

    let site = Span::new(2, 5, FileId::new(1));
    let mut registry = TypeRegistry::new();
    for borrowed in [Some(SelfRef), None] {
        for order in [[Some(SelfValue), borrowed], [borrowed, Some(SelfValue)]] {
            for kind in order {
                registry.record_receiver_kind(site, kind);
            }
            assert!(!registry.consumes_receiver(site));
            registry.clear();
        }
    }
    registry.record_receiver_kind(site, Some(SelfValue));
    assert!(registry.consumes_receiver(site));
    registry.record_receiver_kind(Span::dummy(), Some(SelfValue));
    assert!(!registry.consumes_receiver(Span::dummy()));
    registry.clear();
    assert!(!registry.consumes_receiver(site));
}

#[test]
fn wide_storage_cannot_bypass_the_work_budget() {
    let checker = TypeChecker::with_minimal_context();
    let fields: List<_> = std::iter::repeat_n(Type::Int, MAX_LOAN_SHAPE_STEPS - 1).collect();
    assert!(!checker.carries_stored_loan(&Type::Tuple(fields.clone())));
    let mut oversized = fields;
    oversized.push(Type::Int);
    assert!(checker.carries_stored_loan(&Type::Tuple(oversized)));
    assert!(checker.carries_stored_loan(&Type::Tuple(List::from_iter([
        Type::Int,
        Type::reference(false, Type::Int),
    ]))));
}

#[test]
fn cyclic_bindings_cannot_prove_reference_free_storage() {
    let checker = TypeChecker::with_minimal_context();
    let slot = Type::Var(crate::ty::TypeVar::fresh());
    let scopes: List<Bindings<'_>> = List::from_iter([
        List::from_iter([(
            &slot,
            Node {
                ty: &slot,
                scope: 1,
            },
        )]),
        List::from_iter([(
            &slot,
            Node {
                ty: &slot,
                scope: 0,
            },
        )]),
    ]);
    let mut remaining = MAX_LOAN_SHAPE_STEPS;
    assert!(
        checker
            .loan_node(
                Node {
                    ty: &slot,
                    scope: 0
                },
                &scopes,
                &mut remaining
            )
            .is_none()
    );
    assert_eq!(remaining, 0);
}

#[test]
fn forward_fields_follow_only_the_completed_owner() {
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    for (owner, field) in [("owned", "Int"), ("borrowed", "&Int")] {
        checker.set_current_module_path(owner);
        let module = verum_fast_parser::Parser::new(&format!(
            "type Payload is {{ value: {field} }}; type Parametric<T> is (T);"
        ))
        .parse_module()
        .unwrap();
        let verum_ast::ItemKind::Type(declaration) = &module.items[0].kind else { unreachable!() };
        checker.register_type_name_only(declaration);
        let forward = Type::Placeholder {
            name: format!("{owner}.Payload").into(),
            span: declaration.span,
        };
        assert!(checker.carries_stored_loan(&forward));
        checker.resolve_type_definition(declaration, &mut List::new()).unwrap();
        let verum_ast::ItemKind::Type(generic) = &module.items[1].kind else { unreachable!() };
        checker.resolve_type_definition(generic, &mut List::new()).unwrap();
    }

    for (owner, carries_loan) in [("owned", false), ("borrowed", true), ("absent", true)] {
        let field = Type::Placeholder {
            name: format!("{owner}.Payload").into(),
            span: verum_ast::Span::default(),
        };
        let result = Type::Tuple(List::from_iter([Type::Bool, field]));
        assert_eq!(checker.carries_stored_loan(&result), carries_loan, "{owner}");
    }
    let missing_argument = Type::Placeholder {
        name: "owned.Parametric".into(),
        span: verum_ast::Span::default(),
    };
    assert!(checker.carries_stored_loan(&missing_argument));
}

#[test]
fn declared_tuple_payloads_bind_only_stored_parameters() {
    let module = verum_fast_parser::Parser::new(
        "type Wrapped<T> is (T); type Pair<A, B> is (A, B); type Phantom<T> is ();"
    ).parse_module().unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path("owner");
    assert!(checker.prepare_module_declarations(&module).is_empty());
    for (name, arguments, borrowed) in [
        ("owner.Wrapped", List::from_iter([Type::Int]), false),
        ("owner.Wrapped", List::from_iter([Type::reference(false, Type::Int)]), true),
        ("owner.Pair", List::from_iter([Type::Int, Type::Bool]), false),
        ("owner.Pair", List::from_iter([Type::Int, Type::reference(false, Type::Int)]), true),
        ("owner.Phantom", List::from_iter([Type::reference(false, Type::Int)]), false),
    ] {
        let ty = Type::Named { path: TypeChecker::text_to_path(&name.into()), args: arguments };
        assert_eq!(checker.carries_stored_loan(&ty), borrowed, "{ty:?}");
    }
}
