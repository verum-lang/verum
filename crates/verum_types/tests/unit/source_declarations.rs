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
