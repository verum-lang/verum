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

#[test]
fn scalar_storage_requires_the_exact_registered_language_declaration() {
    let mut checker = TypeChecker::with_minimal_context();
    let scalar = Type::i64_refined(0);
    assert!(checker.carries_stored_loan(&scalar));
    checker.register_primitives();
    for scalar in [scalar, Type::u8_refined(0), Type::u32_refined(0), Type::i128_refined(0)] {
        assert!(!checker.carries_stored_loan(&scalar), "{scalar:?}");
        assert!(!checker.may_supply_call_borrow(&scalar), "{scalar:?}");
        assert!(checker.carries_stored_loan(&Type::reference(false, scalar)));
    }
    checker.set_current_module_path("model");
    let ast = verum_fast_parser::Parser::new(
        "type Int64 is { value: &Int }; type Words is { value: Int64 };"
    ).parse_module().unwrap();
    assert!(checker.prepare_module_declarations(&ast).is_empty());
    for name in ["model.Int64", "model.Words"] {
        let carrier = Type::Named { path: TypeChecker::text_to_path(&name.into()), args: List::new() };
        assert!(checker.carries_stored_loan(&carrier), "{carrier:?}");
    }
}

#[test]
fn owned_container_payloads_keep_real_loans_and_separate_buffer_exposure() {
    for (owner, declaration, scalar_args) in [
        (
            "core.collections.list",
            "type List<T> is {len:Int,cap:Int,ptr:&unsafe T};",
            List::from_iter([Type::Int]),
        ),
        (
            "core.collections.map",
            "type Map<K,V> is {len:Int,cap:Int,entries:&unsafe (K,V),tombstones:Int};",
            List::from_iter([Type::Int, Type::Bool]),
        ),
        (
            "core.collections.deque",
            "type Deque<T> is {data:&unsafe T,head:Int,len:Int,cap:Int};",
            List::from_iter([Type::Int]),
        ),
        (
            "core.collections.set",
            "type Set<T> is {storage:&unsafe T};",
            List::from_iter([Type::Int]),
        ),
    ] {
        let mut checker = TypeChecker::with_minimal_context();
        checker.register_primitives();
        checker.set_current_module_path(owner);
        let module = verum_fast_parser::Parser::new(declaration)
            .parse_module()
            .unwrap();
        assert!(checker.prepare_module_declarations(&module).is_empty());
        let verum_ast::ItemKind::Type(declaration) = &module.items[0].kind else {
            unreachable!()
        };
        let name = format!("{owner}.{}", declaration.name.name);
        let check = |args: List<Type>, stored, writable| {
            let ty = owned_container_type(&name, args);
            assert_eq!(checker.carries_stored_loan(&ty), stored, "{ty:?}");
            assert!(
                checker.may_supply_call_borrow(&ty),
                "backing storage can be exposed: {ty:?}"
            );
            assert_eq!(
                checker.may_store_writable_reference(&ty),
                writable,
                "{ty:?}"
            );
        };
        check(scalar_args.clone(), false, false);
        for position in 0..scalar_args.len() {
            for mutable in [false, true] {
                let mut args = scalar_args.clone();
                args[position] = Type::reference(mutable, Type::Int);
                check(args, true, mutable);
            }
        }
    }
}

#[test]
fn owned_container_authority_does_not_follow_an_unrelated_spelling() {
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path("application");
    let module = verum_fast_parser::Parser::new(
        "type List<T> is {ptr:&unsafe T}; type Map<K,V> is {ptr:&unsafe (K,V)};",
    )
    .parse_module()
    .unwrap();
    assert!(checker.prepare_module_declarations(&module).is_empty());
    for (name, args) in [
        ("application.List", List::from_iter([Type::Int])),
        ("application.Map", List::from_iter([Type::Int, Type::Bool])),
    ] {
        let ty = owned_container_type(name, args);
        assert!(checker.carries_stored_loan(&ty), "{ty:?}");
    }
    let missing = owned_container_type("core.collections.list.List", List::from_iter([Type::Int]));
    assert!(
        checker.carries_stored_loan(&missing),
        "a path alone is not a declaration"
    );
}

fn owned_container_type(name: &str, args: List<Type>) -> Type {
    Type::Named {
        path: TypeChecker::text_to_path(&name.into()),
        args,
    }
}

fn owned_list_checker() -> TypeChecker {
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path("core.collections.list");
    let module = verum_fast_parser::Parser::new("type List<T> is {len:Int,cap:Int,ptr:&unsafe T};")
        .parse_module()
        .unwrap();
    assert!(checker.prepare_module_declarations(&module).is_empty());
    checker
}

#[test]
fn owned_container_nested_values_keep_their_separate_loans() {
    use verum_ast::FileId;
    use verum_modules::{ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
    let mut registry = ModuleRegistry::new();
    for (owner, source) in [
        ("core.collections.list", "module core.collections.list; public type List<T> is {len:Int,cap:Int,ptr:&unsafe T};"),
        ("model", r#"
            module model;
            mount core.collections.list.List;
            type Borrowed is {value:&Int};
            type Owned is {value:Int};
            type Nested<T> is {values:List<T>};
            type Envelope<T> is {values:List<T>, sibling:&Int};
            type Alias<T> is List<T>;
        "#),
    ] {
        let id = registry.allocate_id();
        let file = FileId::new(id.as_u32());
        let module = verum_fast_parser::FastParser::new().parse_module_str(source, file).unwrap();
        let path = ModulePath::from_str(owner);
        let exports = extract_exports_from_module(&module, id, &path).unwrap();
        let mut info = ModuleInfo::new(id, path, module, file, source.into());
        info.exports = exports;
        registry.register(info);
    }
    let module = registry.get_by_path("model").unwrap().ast.clone();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path("model");
    checker.set_module_registry_direct(registry);
    let errors = checker.prepare_module_declarations(&module);
    assert!(errors.is_empty(), "{errors:?}");
    for (name, argument, stored, writable) in [
        ("model.Nested", Type::Int, false, false),
        (
            "model.Nested",
            Type::reference(false, Type::Int),
            true,
            false,
        ),
        ("model.Nested", Type::reference(true, Type::Int), true, true),
        ("model.Envelope", Type::Int, true, false),
        ("model.Alias", Type::Int, false, false),
        ("model.Alias", Type::reference(true, Type::Int), true, true),
        (
            "core.collections.list.List",
            owned_container_type("model.Borrowed", List::new()),
            true,
            false,
        ),
        (
            "core.collections.list.List",
            owned_container_type("model.Owned", List::new()),
            false,
            false,
        ),
        (
            "core.collections.list.List",
            Type::UnsafeReference {
                inner: Box::new(Type::Int),
                mutable: false,
            },
            true,
            false,
        ),
    ] {
        let ty = owned_container_type(name, List::from_iter([argument]));
        assert_eq!(checker.carries_stored_loan(&ty), stored, "{ty:?}");
        assert_eq!(
            checker.may_store_writable_reference(&ty),
            writable,
            "{ty:?}"
        );
        assert!(checker.may_supply_call_borrow(&ty), "{ty:?}");
    }
}

#[test]
fn owned_container_unknown_arguments_and_invalid_rosters_remain_conservative() {
    let checker = owned_list_checker();
    for arguments in [
        List::new(),
        List::from_iter([Type::Int, Type::Bool]),
        List::from_iter([Type::Var(crate::ty::TypeVar::fresh())]),
    ] {
        let ty = owned_container_type("core.collections.list.List", arguments);
        assert!(checker.carries_stored_loan(&ty), "{ty:?}");
        assert!(checker.may_store_writable_reference(&ty), "{ty:?}");
    }
    for source in [
        "type List<T,U> is {ptr:&unsafe T};",
        "type List<T> is &unsafe T;",
    ] {
        let mut checker = TypeChecker::with_minimal_context();
        checker.register_primitives();
        checker.set_current_module_path("core.collections.list");
        let module = verum_fast_parser::Parser::new(source)
            .parse_module()
            .unwrap();
        let errors = checker.prepare_module_declarations(&module);
        assert!(errors.is_empty(), "{errors:?}");
        let arguments = if source.contains("T,U") {
            List::from_iter([Type::Int, Type::Bool])
        } else {
            List::from_iter([Type::Int])
        };
        let ty = owned_container_type("core.collections.list.List", arguments);
        assert!(checker.carries_stored_loan(&ty), "{ty:?}");
    }
}
