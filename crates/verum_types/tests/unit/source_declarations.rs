use super::TypeChecker;
use verum_ast::{ItemKind, cfg::CfgEvaluator};
use verum_fast_parser::Parser;

#[test]
fn lookup_borrows_the_exact_target_selected_declaration() {
    let module = Parser::new(
        "@cfg(target_os = \"linux\") type Payload is { count: Int };
         @cfg(target_os = \"windows\") type Payload is { active: Bool };",
    )
    .parse_module()
    .unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    for (target, index) in [
        ("x86_64-unknown-linux-gnu", 0),
        ("x86_64-pc-windows-msvc", 1),
    ] {
        checker.cfg_evaluator = CfgEvaluator::for_target(target);
        let selected = checker
            .local_source_type_declaration(&module, "Payload")
            .unwrap();
        let ItemKind::Type(original) = &module.items[index].kind else {
            panic!("type declaration")
        };
        assert!(
            std::ptr::eq(selected, original),
            "lookup copied the declaration body"
        );
    }
    checker.cfg_evaluator = CfgEvaluator::for_target("aarch64-apple-darwin");
    assert!(
        checker
            .local_source_type_declaration(&module, "Payload")
            .is_none()
    );
    assert!(
        checker
            .local_source_type_declaration(&module, "Missing")
            .is_none()
    );
}

fn registry_with(owner: &str, source: &str) -> verum_modules::ModuleRegistry {
    use verum_modules::{ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
    let ast = Parser::new(source).parse_module().unwrap();
    let mut registry = ModuleRegistry::new();
    let id = registry.allocate_id();
    let path = ModulePath::from_str(owner);
    let mut module = ModuleInfo::new(
        id,
        path.clone(),
        ast,
        verum_ast::FileId::new(0),
        source.into(),
    );
    module.exports = extract_exports_from_module(&module.ast, id, &path).unwrap();
    registry.register(module);
    registry
}

#[test]
fn registered_lookup_retains_the_exact_declaration_snapshot() {
    let registry = registry_with(
        "scope.provider",
        "public type Contract is protocol {fn value(&self)->Int;};",
    );
    let module = registry.get_by_path("scope.provider").unwrap();
    let ItemKind::Type(original) = &module.ast.items[0].kind else {
        panic!("type")
    };
    let mut checker = TypeChecker::with_minimal_context();
    checker.set_current_module_path("scope.provider");
    checker.set_module_registry_direct(registry);
    for _ in 0..3 {
        let (selected, owner) = checker
            .declaring_source_type_declaration("Contract")
            .unwrap();
        assert_eq!(owner.as_str(), "scope.provider");
        assert!(
            std::ptr::eq(&selected.body, &original.body),
            "lookup copied the protocol declaration"
        );
    }
}

#[test]
fn local_scope_lookup_retains_its_exact_declaration_snapshot() {
    let module = verum_common::Shared::new(
        Parser::new("type Item is {value:Int};")
            .parse_module()
            .unwrap(),
    );
    let ItemKind::Type(original) = &module.items[0].kind else {
        panic!("type")
    };
    let mut checker = TypeChecker::with_minimal_context();
    checker.set_current_module_path("scope.local");
    checker
        .source_type_registration
        .local_declarations
        .push(("scope.local".into(), module.clone()));
    let (selected, owner) = checker.declaring_source_type_declaration("Item").unwrap();
    checker.source_type_registration.local_declarations.pop();
    assert_eq!(owner.as_str(), "scope.local");
    assert!(
        std::ptr::eq(&selected.body, &original.body),
        "lookup copied a retained lexical declaration"
    );
}

#[test]
fn a_new_registry_generation_cannot_reuse_the_old_declaration() {
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path("scope.provider");
    let old_registry = registry_with("scope.provider", "public type Item is {public value:Int};");
    let old_module = old_registry.get_by_path("scope.provider").unwrap();
    checker.set_module_registry_direct(old_registry);
    let (old, owner) = checker.declaring_source_type_declaration("Item").unwrap();
    checker
        .load_source_type_declaration_with_parameter_key(&old, &owner, old.span)
        .unwrap();
    let registry = registry_with("scope.provider", "public type Item is {public value:Bool};");
    let module = registry.get_by_path("scope.provider").unwrap();
    checker.set_module_registry_direct(registry);
    let (new, owner) = checker.declaring_source_type_declaration("Item").unwrap();
    let ItemKind::Type(original) = &old_module.ast.items[0].kind else {
        panic!("old type")
    };
    let ItemKind::Type(replacement) = &module.ast.items[0].kind else {
        panic!("new type")
    };
    assert!(std::ptr::eq(&old.body, &original.body));
    assert!(std::ptr::eq(&new.body, &replacement.body));
    assert_ne!(old.body, new.body);
    let error = checker
        .load_source_type_declaration_with_parameter_key(&new, &owner, new.span)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("conflicting source declarations"),
        "{error:?}"
    );
}

#[test]
fn alias_rebinding_uses_the_new_owner_without_changing_a_retained_snapshot() {
    use verum_modules::{ModuleInfo, ModulePath, extract_exports_from_module};
    let mut registry = registry_with("scope.left", "public type Item is {public value:Int};");
    let ast = Parser::new("public type Item is {public value:Bool};")
        .parse_module()
        .unwrap();
    let id = registry.allocate_id();
    let path = ModulePath::from_str("scope.right");
    let mut right = ModuleInfo::new(id, path.clone(), ast, verum_ast::FileId::new(1), "".into());
    right.exports = extract_exports_from_module(&right.ast, id, &path).unwrap();
    registry.register(right);
    registry.register_path_alias("selected", "scope.left");
    let checker = TypeChecker::with_minimal_context();
    let left = registry.get_by_path_aliased("selected").unwrap();
    let (old, old_owner) = checker
        .find_type_declaration_with_source_module(&left.ast, "Item", &"selected".into(), &registry)
        .unwrap();
    let (observed, reads) = std::sync::Arc::new(registry.clone()).observe_reads();
    let _ = checker
        .find_type_declaration_with_source_module(&left.ast, "Item", &"selected".into(), &observed)
        .unwrap();
    let mut next = registry.clone();
    next.register_path_alias("selected", "scope.right");
    assert!(
        !reads.unchanged(&next),
        "alias selection must remain observable"
    );
    let right = next.get_by_path_aliased("selected").unwrap();
    let (new, new_owner) = checker
        .find_type_declaration_with_source_module(&right.ast, "Item", &"selected".into(), &next)
        .unwrap();
    assert_eq!(old_owner.as_str(), "scope.left");
    assert_eq!(new_owner.as_str(), "scope.right");
    let ItemKind::Type(original) = &left.ast.items[0].kind else {
        panic!("left type")
    };
    let ItemKind::Type(replacement) = &right.ast.items[0].kind else {
        panic!("right type")
    };
    assert!(std::ptr::eq(&old.body, &original.body));
    assert!(std::ptr::eq(&new.body, &replacement.body));
    assert_ne!(old.body, new.body);
}

#[test]
fn a_standalone_ast_cannot_borrow_an_unrelated_registered_body() {
    let registry = registry_with("scope.provider", "public type Item is {public value:Int};");
    let checker = TypeChecker::with_minimal_context();
    let selected = {
        let ast = Parser::new("public type Item is {public value:Bool};")
            .parse_module()
            .unwrap();
        let (selected, owner) = checker
            .find_type_declaration_with_source_module(
                &ast,
                "Item",
                &"scope.provider".into(),
                &registry,
            )
            .unwrap();
        let ItemKind::Type(declaration) = &ast.items[0].kind else {
            panic!("lexical type")
        };
        assert_eq!(selected.body, declaration.body);
        assert_eq!(owner.as_str(), "scope.provider");
        selected
    };
    let registered = registry.get_by_path("scope.provider").unwrap();
    let ItemKind::Type(declaration) = &registered.ast.items[0].kind else {
        panic!("registered type")
    };
    assert_ne!(selected.body, declaration.body);
}

#[test]
fn public_reexports_select_the_exact_target_type_attributes() {
    use verum_modules::{ModuleInfo, ModulePath, extract_exports_from_module};
    let mut registry = registry_with(
        "scope.provider",
        r#"
        @cfg(target_os = "linux") public type Payload is { count:Int };
        @cfg(target_os = "windows") public type Payload is { active:Bool };
    "#,
    );
    let ast = Parser::new("public mount scope.provider.{Payload as Published};")
        .parse_module()
        .unwrap();
    let id = registry.allocate_id();
    let path = ModulePath::from_str("scope.facade");
    let mut facade = ModuleInfo::new(id, path.clone(), ast, verum_ast::FileId::new(1), "".into());
    facade.exports = extract_exports_from_module(&facade.ast, id, &path).unwrap();
    registry.register(facade);
    let provider = registry.get_by_path("scope.provider").unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    for (target, index) in [
        ("x86_64-pc-windows-msvc", 1),
        ("x86_64-unknown-linux-gnu", 0),
    ] {
        checker.cfg_evaluator = CfgEvaluator::for_target(target);
        let ItemKind::Type(original) = &provider.ast.items[index].kind else {
            panic!("target type")
        };
        for (entry, name) in [("scope.provider", "Payload"), ("scope.facade", "Published")] {
            let (selected, owner) = checker
                .resolve_public_source_type(entry, name, &registry)
                .expect("the active public declaration");
            assert_eq!(owner.as_str(), "scope.provider");
            assert_eq!(selected, *original, "entry={entry}, target={target}");
        }
    }
    checker.cfg_evaluator = CfgEvaluator::for_target("aarch64-apple-darwin");
    for (entry, name) in [("scope.provider", "Payload"), ("scope.facade", "Published")] {
        assert!(
            checker
                .resolve_public_source_type(entry, name, &registry)
                .is_none(),
            "inactive type from {entry}"
        );
    }
}

#[test]
fn an_active_private_type_cannot_borrow_an_inactive_public_export() {
    let registry = registry_with(
        "scope.provider",
        r#"
        @cfg(target_os = "linux") public type Payload is { count:Int };
        @cfg(target_os = "windows") type Payload is { active:Bool };
    "#,
    );
    let mut checker = TypeChecker::with_minimal_context();
    checker.cfg_evaluator = CfgEvaluator::for_target("x86_64-pc-windows-msvc");
    assert!(
        checker
            .resolve_public_source_type("scope.provider", "Payload", &registry)
            .is_none()
    );
    checker.cfg_evaluator = CfgEvaluator::for_target("x86_64-unknown-linux-gnu");
    assert!(
        checker
            .resolve_public_source_type("scope.provider", "Payload", &registry)
            .is_some()
    );
}

fn add_dependency_module(registry: &mut verum_modules::ModuleRegistry, owner: &str, source: &str) {
    use verum_modules::{ModuleInfo, ModulePath, extract_exports_from_module};
    let ast = Parser::new(source).parse_module().unwrap();
    let id = registry.allocate_id();
    let path = ModulePath::from_str(owner);
    let mut module = ModuleInfo::new(
        id,
        path.clone(),
        ast,
        verum_ast::FileId::new(id.as_u32()),
        source.into(),
    );
    module.exports = extract_exports_from_module(&module.ast, id, &path).unwrap();
    registry.register(module);
}

fn source_dependency_registry() -> verum_modules::ModuleRegistry {
    let mut registry = registry_with(
        "scope.carrier",
        "mount selected.Value; public type Carrier is {public value:Value};",
    );
    add_dependency_module(
        &mut registry,
        "scope.left",
        "public type Value is {public number:Int};",
    );
    add_dependency_module(
        &mut registry,
        "scope.right",
        "public type Value is {public flag:Bool};",
    );
    registry.register_path_alias("selected", "scope.left");
    registry
}

fn dependency_checker(registry: verum_modules::ModuleRegistry) -> TypeChecker {
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path("scope.carrier");
    checker.set_module_registry_direct(registry);
    checker
}

fn load_carrier(checker: &mut TypeChecker) -> crate::Result<(crate::Type, verum_common::Text)> {
    let (declaration, owner) = checker
        .declaring_source_type_declaration("Carrier")
        .unwrap();
    checker.load_source_type_declaration_with_parameter_key(&declaration, &owner, declaration.span)
}

#[test]
fn unchanged_source_carrier_cannot_reuse_a_rebound_dependency_alias() {
    let registry = source_dependency_registry();
    let mut checker = dependency_checker(registry.clone());
    let active = verum_common::Shared::new(parking_lot::RwLock::new(registry));
    checker.set_module_registry(active.clone());
    load_carrier(&mut checker).unwrap();
    let fields = "__struct_fields_scope.carrier.Carrier";
    let before = checker.resolved_type(fields).unwrap().clone();
    active
        .write()
        .register_path_alias("selected", "scope.right");
    let mut fresh = dependency_checker(active.read().clone());
    load_carrier(&mut fresh).unwrap();
    assert_ne!(
        &before,
        fresh.resolved_type(fields).unwrap(),
        "the new dependency must change the checked field type"
    );
    assert!(
        load_carrier(&mut checker).is_err(),
        "an unchanged AST reused a type checked against the old dependency alias"
    );
}

#[test]
fn source_record_extent_is_bound_to_the_constant_in_its_checked_snapshot() {
    let provider = |width| registry_with("scope.carrier", &format!(
        "public type Carrier is {{public data:[Int;Width]}}; const Width:Int={width};"
    ));
    let active = verum_common::Shared::new(parking_lot::RwLock::new(provider(3)));
    let mut checker = dependency_checker(active.read().clone());
    checker.set_module_registry(active.clone());
    let extent = |checker: &TypeChecker| match checker.local_record_field("Carrier", "data") {
        Some(crate::Type::Array { size, .. }) => *size,
        other => panic!("expected checked array, got {other:?}"),
    };
    load_carrier(&mut checker).unwrap();
    assert_eq!(extent(&checker), Some(3));

    *active.write() = provider(7);
    let mut fresh = dependency_checker(active.read().clone());
    load_carrier(&mut fresh).unwrap();
    assert_eq!(extent(&fresh), Some(7));
    let stale = load_carrier(&mut checker).expect_err("an old field accepted a new constant");
    assert!(stale.to_string().contains("source dependencies changed"), "{stale}");
}

#[test]
fn unrelated_module_addition_preserves_a_completed_source_carrier() {
    let registry = source_dependency_registry();
    let mut checker = dependency_checker(registry.clone());
    let before = load_carrier(&mut checker).unwrap();
    assert_eq!(load_carrier(&mut checker).unwrap(), before);
    let mut next = registry.clone();
    add_dependency_module(
        &mut next,
        "scope.unrelated",
        "public type Unrelated is {public text:Text};",
    );
    next.register_path_alias("unrelated_alias", "scope.unrelated");
    checker.set_module_registry_direct(next);
    assert_eq!(load_carrier(&mut checker).unwrap(), before);
    assert!(
        checker
            .source_type_registration.is_completed(&"scope.carrier.Carrier".into())
    );
}

fn check_source(
    checker: &mut TypeChecker,
    owner: &str,
    source: &str,
) -> verum_common::List<verum_common::Text> {
    let module = Parser::new(source).parse_module().unwrap();
    checker.set_current_module_path(owner);
    let mut errors = checker.prepare_module_declarations(&module);
    if errors.is_empty() {
        for item in &module.items {
            if let Err(error) = checker.check_item(item) {
                errors.push(error);
            }
        }
    }
    errors.extend(checker.take_deferred_errors());
    errors
        .iter()
        .chain(checker.diagnostic_sources().iter())
        .map(|error| error.to_string().into())
        .collect()
}

#[test]
fn ordinary_consumer_cannot_accept_an_obsolete_imported_field() {
    let registry = source_dependency_registry();
    let active = verum_common::Shared::new(parking_lot::RwLock::new(registry.clone()));
    let mut checker = dependency_checker(registry);
    checker.set_module_registry(active.clone());
    let old_consumer = "mount scope.carrier.Carrier; fn old_field(v:Carrier)->Int {v.value.number}";
    let new_consumer = "mount scope.carrier.Carrier; fn new_field(v:Carrier)->Bool {v.value.flag}";
    let initial = check_source(&mut checker, "app.first", old_consumer);
    assert!(initial.is_empty(), "{initial:?}");
    active
        .write()
        .register_path_alias("selected", "scope.right");
    let updated = active.read().clone();
    let current = check_source(
        &mut dependency_checker(updated.clone()),
        "app.new",
        new_consumer,
    );
    assert!(current.is_empty(), "{current:?}");
    assert!(!check_source(&mut dependency_checker(updated), "app.old", old_consumer).is_empty());
    let stale = check_source(&mut checker, "app.old", old_consumer);
    assert!(
        stale
            .iter()
            .any(|error| error.to_string().contains("source dependencies changed")),
        "{stale:?}"
    );
}

#[test]
fn outer_carrier_retains_transitive_dependencies_of_a_cached_child() {
    let mut registry = source_dependency_registry();
    add_dependency_module(
        &mut registry,
        "scope.outer",
        "mount scope.carrier.Carrier; public type Outer is {public child:Carrier};",
    );
    let mut checker = dependency_checker(registry.clone());
    load_carrier(&mut checker).unwrap();
    checker.set_current_module_path("scope.outer");
    let (declaration, owner) = checker.declaring_source_type_declaration("Outer").unwrap();
    let load = |checker: &mut TypeChecker| {
        checker.load_source_type_declaration_with_parameter_key(
            &declaration,
            &owner,
            declaration.span,
        )
    };
    load(&mut checker).unwrap();
    let mut broken = registry.clone();
    broken.register_path_alias("selected", "missing");
    checker.set_module_registry_direct(broken);
    let error =
        load(&mut checker).expect_err("an outer field depended on the previously accepted child");
    assert!(error.to_string().contains("scope.outer.Outer"), "{error}");
    checker.set_module_registry_direct(registry);
    load(&mut checker).expect("restoring the observed inputs preserves the completed declaration");
}

#[test]
fn source_retry_after_an_unavailable_dependency_publishes_only_the_new_evidence() {
    let mut registry = source_dependency_registry();
    registry.register_path_alias("selected", "scope.absent");
    let mut checker = dependency_checker(registry.clone());
    assert!(load_carrier(&mut checker).is_err());
    assert!(
        !checker
            .source_type_registration.is_completed(&"scope.carrier.Carrier".into())
    );
    add_dependency_module(
        &mut registry,
        "scope.absent",
        "public type Value is {public count:Int};",
    );
    checker.set_module_registry_direct(registry);
    load_carrier(&mut checker).unwrap();
    let errors = check_source(
        &mut checker,
        "app",
        "mount scope.carrier.Carrier; fn read(v:Carrier)->Int {v.value.count}",
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn replacing_the_exact_dependency_body_invalidates_an_unchanged_carrier() {
    let original = source_dependency_registry();
    let active = verum_common::Shared::new(parking_lot::RwLock::new(original.clone()));
    let mut checker = dependency_checker(original);
    checker.set_module_registry(active.clone());
    load_carrier(&mut checker).unwrap();
    let before = active.read().get_by_path("scope.carrier").unwrap();
    {
        let mut registry = active.write();
        let keep = ["scope.carrier".to_owned(), "scope.right".to_owned()]
            .into_iter()
            .collect();
        assert_eq!(registry.retain_paths(&keep), (2, 1));
        add_dependency_module(
            &mut registry,
            "scope.left",
            "public type Value is {public count:Int};",
        );
    }
    let after = active.read().get_by_path("scope.carrier").unwrap();
    assert!(std::ptr::eq(&*before, &*after));
    assert_eq!(
        active
            .read()
            .get_by_path_aliased("selected")
            .unwrap()
            .path
            .to_string(),
        "scope.left"
    );
    let error = load_carrier(&mut checker).unwrap_err();
    assert!(
        error.to_string().contains("scope.carrier.Carrier"),
        "{error}"
    );
    let mut fresh = dependency_checker(active.read().clone());
    let errors = check_source(
        &mut fresh,
        "app",
        "mount scope.carrier.Carrier; fn read(v:Carrier)->Int {v.value.count}",
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn checked_nested_aliases_stay_private_and_expire_with_their_transaction() {
    use verum_ast::Span;
    use crate::{Type, TypeError};
    let ast = Parser::new("type Key is Int { self > 0 }; type Outer is { key: Key };")
        .parse_module().unwrap();
    let ItemKind::Type(key) = &ast.items[0].kind else { panic!("expected Key declaration") };
    let ItemKind::Type(outer) = &ast.items[1].kind else { panic!("expected Outer declaration") };
    for commit in [false, true] {
        let mut checker = TypeChecker::with_minimal_context();
        checker.register_primitives();
        checker.set_current_module_path("sample");
        let subject = Type::Named { path: TypeChecker::text_to_path(&"sample.Key".into()), args: Default::default() };
        let result = checker.with_source_type_registration(outer, "sample.Outer".into(), |checker| {
            checker.register_type_declaration(key)?;
            assert!(matches!(checker.staged_proof_receiver(&subject, key.span)?, Some(Type::Refined { .. })));
            if commit { Ok(()) } else { Err(TypeError::TypeNotFound { name: "Missing".into(), span: Span::default() }) }
        });
        assert_eq!(result.is_ok(), commit);
        assert!(checker.staged_proof_receiver(&subject, key.span).unwrap().is_none());
    }
}
