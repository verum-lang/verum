//! Source declaration identity follows finite import graphs on ordinary stacks.
use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::VerumParser;
use verum_modules::{ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
use verum_types::TypeChecker;

fn register(registry: &mut ModuleRegistry, owner: &str, source: &str) {
    let id = registry.allocate_id();
    let file = FileId::new(id.as_u32());
    let ast = VerumParser::new().parse_module_str(source, file).unwrap();
    let path = ModulePath::from_str(owner);
    let exports = extract_exports_from_module(&ast, id, &path).unwrap();
    let mut info = ModuleInfo::new(id, path, ast, file, source.into());
    info.exports = exports;
    registry.register(info);
}

fn chain(depth: usize, terminal: &str) -> ModuleRegistry {
    let mut registry = ModuleRegistry::new();
    for index in 0..depth {
        register(
            &mut registry,
            &format!("graph.node{index}"),
            &format!(
                "public mount graph.node{}.{{Symbol{} as Symbol{index}}};",
                index + 1,
                index + 1,
            ),
        );
    }
    register(&mut registry, &format!("graph.node{depth}"), terminal);
    registry.register_path_alias("entry", "graph.node0");
    registry
}

#[test]
fn deep_renamed_reexports_retain_the_exact_declaring_owner() {
    std::thread::spawn(|| {
        const DEPTH: usize = 4096;
        let registry = chain(
            DEPTH,
            &format!("public type Symbol{DEPTH} is {{ public value: Int }};"),
        );
        let checker = TypeChecker::with_minimal_context();
        for entry in ["graph.node0", "entry"] {
            let (declaration, owner) = checker
                .resolve_public_source_type(entry, "Symbol0", &registry)
                .expect("finite valid re-export chain");
            assert_eq!(
                declaration.name.name.as_str(),
                format!("Symbol{DEPTH}").as_str()
            );
            assert_eq!(owner.as_str(), format!("graph.node{DEPTH}").as_str());
        }
    })
    .join()
    .unwrap();
}

#[test]
fn deep_cycles_missing_and_private_targets_do_not_borrow_homonyms() {
    std::thread::spawn(|| {
        const DEPTH: usize = 1024;
        for terminal in [
            format!("public mount entry.{{Symbol0 as Symbol{DEPTH}}};"),
            format!("public mount graph.absent.{{Symbol{DEPTH}}};"),
            format!("type Symbol{DEPTH} is {{ public value: Int }};"),
        ] {
            let mut registry = chain(DEPTH, &terminal);
            register(
                &mut registry,
                "graph",
                &format!("public type Symbol{DEPTH} is {{ value: Bool }};"),
            );
            register(
                &mut registry,
                "unrelated",
                "public type Symbol0 is { value: Bool };",
            );
            let checker = TypeChecker::with_minimal_context();
            assert!(
                checker
                    .resolve_public_source_type("entry", "Symbol0", &registry)
                    .is_none(),
                "{terminal}"
            );
        }
    })
    .join()
    .unwrap();
}

fn check(registry: &ModuleRegistry, owner: &str) -> List<Text> {
    let module = registry.get_by_path(owner).unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_module_registry_direct(registry.clone());
    checker.set_current_module_path(owner);
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&module.ast)
        .into_iter()
        .map(|error| error.to_string().into())
        .collect();
    for item in &module.ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error.to_string().into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| error.to_string().into()),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| error.to_string().into()),
    );
    errors
}

#[test]
fn source_local_private_types_stay_available_to_their_public_function_signatures() {
    let mut registry = ModuleRegistry::new();
    register(
        &mut registry,
        "provider",
        "type Hidden is { public value: Int }; public fn create() -> Hidden { Hidden { value: 73 } }",
    );
    register(
        &mut registry,
        "unrelated",
        "public type Hidden is { public value: Bool };",
    );
    register(
        &mut registry,
        "consumer",
        "mount provider.create; fn read() -> Int { create().value }",
    );
    for owner in ["provider", "consumer"] {
        let errors = check(&registry, owner);
        assert!(errors.is_empty(), "{owner}: {errors:?}");
    }
    assert!(
        TypeChecker::with_minimal_context()
            .resolve_public_source_type("provider", "Hidden", &registry)
            .is_none()
    );
}

#[test]
fn resolved_export_origins_precede_source_order_mount_fallbacks() {
    for (source, before) in [
        (
            "public mount providers.early.*; public mount providers.late.Token;",
            "providers.early",
        ),
        (
            "public mount providers.late.Token; public mount providers.early.*;",
            "providers.late",
        ),
        (
            "public mount providers.{early.*, late.Token};",
            "providers.early",
        ),
    ] {
        let mut registry = ModuleRegistry::new();
        for owner in ["providers.early", "providers.late"] {
            register(
                &mut registry,
                owner,
                "public type Token is { public value: Int };",
            );
        }
        register(&mut registry, "facade", source);
        let checker = TypeChecker::with_minimal_context();
        // Source admission can query before all re-export identities have been
        // resolved. The placeholder's AST traversal retains source order.
        assert_eq!(
            checker
                .resolve_public_source_type("facade", "Token", &registry)
                .unwrap()
                .1
                .as_str(),
            before,
            "{source}"
        );
        verum_modules::resolve_specific_reexport_kinds(&mut registry).unwrap();
        // Once the explicit export has an origin, it is authoritative even if
        // another mount appears first. No metadata is forged by the test.
        assert_eq!(
            checker
                .resolve_public_source_type("facade", "Token", &registry)
                .unwrap()
                .1
                .as_str(),
            "providers.late",
            "{source}"
        );
    }
}

#[test]
fn local_declarations_precede_source_mounts() {
    let mut registry = ModuleRegistry::new();
    register(
        &mut registry,
        "dependency",
        "public type Token is { public value: Bool };",
    );
    register(
        &mut registry,
        "facade",
        "public mount dependency.*; public type Token is { public value: Int };",
    );
    let checker = TypeChecker::with_minimal_context();
    assert_eq!(
        checker
            .resolve_public_source_type("facade", "Token", &registry)
            .unwrap()
            .1
            .as_str(),
        "facade"
    );
}

#[test]
fn only_explored_mount_alternatives_contribute_registry_observations() {
    for (source, explored) in [
        (
            "public mount selected.Token; public mount alternative.*;",
            false,
        ),
        (
            "public mount alternative.*; public mount selected.Token;",
            true,
        ),
    ] {
        let mut registry = ModuleRegistry::new();
        register(
            &mut registry,
            "selected",
            "public type Token is { public value: Int };",
        );
        register(&mut registry, "facade", source);
        let base = std::sync::Arc::new(registry);
        let (observed, receipt) = base.observe_reads();
        let checker = TypeChecker::with_minimal_context();
        assert_eq!(
            checker
                .resolve_public_source_type("facade", "Token", &observed)
                .unwrap()
                .1
                .as_str(),
            "selected"
        );
        let mut changed = (*base).clone();
        register(
            &mut changed,
            "alternative",
            "public type Token is { public value: Bool };",
        );
        assert_eq!(receipt.unchanged(&changed), !explored, "{source}");
        assert_eq!(
            checker
                .resolve_public_source_type("facade", "Token", &changed)
                .unwrap()
                .1
                .as_str(),
            if explored { "alternative" } else { "selected" }
        );
    }
}

#[test]
fn grouped_imports_typecheck_against_complete_source_owners() {
    for (mount, first, second) in [
        (
            "mount providers.{early.Token as First, late.Token as Second};",
            "First",
            "Second",
        ),
        (
            "mount providers.{early.{Token as First}, late.{Token as Second}};",
            "First",
            "Second",
        ),
        ("mount providers.{early.*, flag.*};", "Token", "Flag"),
    ] {
        let mut registry = ModuleRegistry::new();
        register(
            &mut registry,
            "providers.early",
            "public type Token is { public value: Int };",
        );
        register(
            &mut registry,
            "providers.late",
            "public type Token is { public value: Bool };",
        );
        register(
            &mut registry,
            "providers",
            "public type Token is { public value: Float };",
        );
        register(
            &mut registry,
            "providers.flag",
            "public type Flag is { public value: Bool };",
        );
        register(
            &mut registry,
            "consumer",
            &format!(
                "{mount} fn first(value: {first}) -> Int {{ value.value }} fn second(value: {second}) -> Bool {{ value.value }}"
            ),
        );
        let errors = check(&registry, "consumer");
        assert!(errors.is_empty(), "{mount}: {errors:?}");
    }
}

#[test]
fn grouped_type_imports_refuse_missing_and_private_leaf_owners() {
    for provider in [None, Some("type Token is { public value: Bool };")] {
        let mut registry = ModuleRegistry::new();
        register(
            &mut registry,
            "providers",
            "public type Token is { public value: Int };",
        );
        if let Some(source) = provider {
            register(&mut registry, "providers.owned", source);
        }
        register(
            &mut registry,
            "consumer",
            "mount providers.{owned.Token as Hidden}; fn identity(value: Hidden) -> Hidden { value }",
        );
        let errors = check(&registry, "consumer");
        assert!(
            !errors.is_empty(),
            "a parent homonym must not supply {provider:?}"
        );
    }
}

#[test]
fn builtin_aliases_cannot_hide_missing_or_private_declarations() {
    for provider in ["", "type Absent is { value: Bool };"] {
        let mut registry = ModuleRegistry::new();
        register(&mut registry, "provider", provider);
        register(&mut registry, "consumer", "mount provider.{Absent as Int};");
        assert!(!check(&registry, "consumer").is_empty(), "{provider}");
    }
}

#[test]
fn grouped_self_alias_requires_an_accessible_namespace() {
    for parent in [None, Some("module hidden;"), Some("public module hidden;")] {
        let mut registry = ModuleRegistry::new();
        if let Some(source) = parent {
            register(&mut registry, "provider", source);
            register(&mut registry, "provider.hidden", "public fn make() -> Int { 7 }");
        }
        register(&mut registry, "consumer", "mount provider.hidden.{self as service};");
        let errors = check(&registry, "consumer");
        assert_eq!(errors.is_empty(), parent == Some("public module hidden;"), "{parent:?}: {errors:?}");
    }
}

#[test]
fn grouped_self_alias_dispatches_the_namespace_without_importing_its_items() {
    let mut registry = ModuleRegistry::new();
    register(&mut registry, "provider", "public fn make() -> Int { 7 }");
    register(&mut registry, "consumer", "mount provider.{self as service}; fn check() -> Int { service.make() }");
    let errors = check(&registry, "consumer");
    assert!(errors.is_empty(), "{errors:?}");
    register(&mut registry, "bare", "mount provider.{self as service}; fn check() -> Int { make() }");
    assert!(!check(&registry, "bare").is_empty());
}
