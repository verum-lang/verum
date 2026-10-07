//! Value bindings keep precedence over same-spelled type property receivers.
use verum_ast::ItemKind;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn check(source: &str) {
    let errors = errors("fixture", source);
    assert!(errors.is_empty(), "{source}: {errors:?}");
}

fn errors(owner: &str, source: &str) -> List<Text> {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path(owner);
    for item in &ast.items {
        match &item.kind {
            ItemKind::Type(decl) => checker.register_type_declaration(decl).unwrap(),
            ItemKind::Function(decl) => checker.register_function_signature(decl).unwrap(),
            _ => panic!("fixture declaration"),
        }
    }
    let mut errors: List<Text> = ast
        .items
        .iter()
        .filter_map(|item| {
            checker
                .check_item(item)
                .err()
                .map(|error| Text::from(format!("{error:?}")))
        })
        .collect();
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors
}

#[test]
fn value_property_uses_declared_field_type_for_any_binding_case() {
    for binding in ["cell", "Cell", "Int"] {
        for access in [
            binding.to_owned(),
            format!("({binding})"),
            format!("(&{binding})"),
        ] {
            check(&format!(
                "type Cell is {{ size: Bool }}; fn probe() -> Bool {{ let {binding} = Cell {{ size: true }}; {access}.size }}"
            ));
            check(&format!(
                "type Cell is {{ size: Bool }}; fn probe({binding}: Cell) -> Bool {{ {access}.size }}"
            ));
            check(&format!(
                "type Cell is {{ size: Bool }}; fn probe(mut {binding}: Cell) -> Bool {{ {access}.size }}"
            ));
        }
    }
}

#[test]
fn declaration_properties_preserve_their_declared_results() {
    for source in [
        "type Cell is { size: Bool }; fn probe() -> Int { Cell.size }",
        "fn probe<T>() -> Int { T.size }",
        "fn probe<T>() -> Int { (&T).size }",
        "fn probe() -> Int { (&checked Byte).size }",
        "fn probe() -> Int { Int.size }",
        "fn probe() -> Bool { Int.is_signed }",
        "fn probe() -> Text { Int.name }",
    ] {
        check(source);
    }
}

#[test]
fn structural_type_properties_require_declared_element_operands() {
    for receiver in [
        "[Byte]",
        "[Byte; 3]",
        "(&[Byte])",
        "(&checked [Byte; 3])",
        "(&unsafe [Byte])",
    ] {
        check(&format!("fn probe() -> Int {{ {receiver}.size }}"));
    }
    // Byte here is a Bool value; the array's element type must remain Bool.
    check("fn probe() -> Bool { let Byte = true; let values = (&[Byte]).min; values[0] }");
    check("fn probe() -> Bool { let Byte = true; let values = (&[Byte; 3]).min; values[0] }");
}

#[test]
fn qualified_declaration_properties_and_local_root_values_keep_separate_owners() {
    for source in [
        "type Cell is { size: Bool }; fn probe() -> Int { fixture.Cell.size }",
        "type Cell is { size: Bool }; fn probe() -> Int { (fixture.Cell).size }",
        "type Cell is { size: Bool }; fn probe() -> Int { (&fixture.Cell).size }",
        "type Cell is { size: Bool }; type Root is { Cell: Cell }; fn probe() -> Bool { let fixture = Root { Cell: Cell { size: true } }; fixture.Cell.size }",
    ] {
        check(source);
    }
    let problems = errors(
        "fixture",
        "fn probe() -> Int { let Byte = true; (&unsafe [Byte]).size }",
    );
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("unsafe reference requires unsafe block")),
        "{problems:?}"
    );
}

fn module_errors(mut checker: TypeChecker, source: &str, preload: bool) -> List<Text> {
    let ast = Parser::new(source)
        .parse_module()
        .expect("module source grammar");
    checker.register_primitives();
    // The production loader supplies the module registry before processing
    // mounts. Keep actual source module bodies, not injected type bindings.
    let mut registry = verum_modules::ModuleRegistry::new();
    registry.register(verum_modules::ModuleInfo::new(
        verum_modules::ModuleId::new(0),
        verum_modules::ModulePath::from_str("cog"),
        ast.clone(),
        verum_ast::FileId::new(0),
        source.into(),
    ));
    for (index, item) in ast.items.iter().enumerate() {
        if let ItemKind::Module(module) = &item.kind {
            if let Some(items) = &module.items {
                let mut body = ast.clone();
                body.items = items.clone();
                registry.register(verum_modules::ModuleInfo::new(
                    verum_modules::ModuleId::new((index + 1) as u32),
                    verum_modules::ModulePath::from_str(module.name.name.as_str()),
                    body,
                    verum_ast::FileId::new(0),
                    source.into(),
                ));
            }
        }
    }
    checker.set_module_registry_direct(registry);
    if preload {
        checker.register_stdlib_types_for_module(&ast);
    }
    let mut errors: List<Text> = ast
        .items
        .iter()
        .filter_map(|item| {
            checker
                .check_item(item)
                .err()
                .map(|error| Text::from(format!("{error:?}")))
        })
        .collect();
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors
}

#[test]
fn module_alias_properties_use_the_actual_type_declaration() {
    for receiver in [
        "short.Cell",
        "(&unsafe short.Cell)",
        "[short.Cell; 3]",
        "(&checked [short.Cell])",
    ] {
        let source = format!(
            "module alpha {{ public type Cell is {{ size: Bool }}; }} mount alpha as short; fn probe() -> Int {{ {receiver}.size }}"
        );
        let problems = module_errors(TypeChecker::new(), &source, false);
        assert!(problems.is_empty(), "{source}: {problems:?}");
    }
    let source = "module alpha { public type Cell is { size: Bool }; } mount alpha as short; type Root is { Cell: alpha.Cell }; fn probe(short: Root) -> Bool { short.Cell.size }";
    assert!(module_errors(TypeChecker::new(), source, false).is_empty());
}

fn property_metadata() -> verum_types::core_metadata::CoreMetadata {
    use verum_common::{Maybe, ResourceDiscipline};
    use verum_types::core_metadata::*;
    let mut metadata = CoreMetadata::default();
    for owner in ["core.alpha", "core.beta"] {
        metadata.types.insert(
            format!("{owner}.Cell").into(),
            TypeDescriptor {
                name: "Cell".into(),
                module_path: owner.into(),
                origin_module_path: Maybe::None,
                generic_params: List::new(),
                kind: TypeDescriptorKind::Record {
                    fields: List::new(),
                },
                size: Maybe::Some(8),
                alignment: Maybe::Some(8),
                methods: List::new(),
                implements: List::new(),
                decl_span: Maybe::None,
                is_public: true,
                is_transparent_wrapper: false,
                resource_discipline: ResourceDiscipline::Unrestricted,
            },
        );
    }
    metadata
}

#[test]
fn metadata_properties_resolve_exact_module_aliases_without_eager_type_keys() {
    for preload in [false, true] {
        let metadata = std::sync::Arc::new(property_metadata());
        for receiver in [
            "core.alpha.Cell",
            "short.Cell",
            "(&unsafe short.Cell)",
            "[short.Cell; 3]",
        ] {
            let source =
                format!("mount core.alpha as short; fn probe() -> Int {{ {receiver}.size }}");
            let problems = module_errors(
                TypeChecker::new_with_core(metadata.clone()),
                &source,
                preload,
            );
            assert!(
                problems.is_empty(),
                "preload={preload} {source}: {problems:?}"
            );
        }
        for receiver in ["short.Missing", "absent.Cell"] {
            let source = format!(
                "mount core.alpha as short; fn probe() -> Int {{ (&unsafe {receiver}).size }}"
            );
            assert!(
                !module_errors(
                    TypeChecker::new_with_core(metadata.clone()),
                    &source,
                    preload
                )
                .is_empty(),
                "unknown owner must not use a same-leaf descriptor: {source}"
            );
        }
    }
}

#[test]
fn associated_type_properties_require_declared_projections() {
    for source in [
        "type Source is protocol { type Item; }; fn probe<T: Source>() -> Int { T.Item.size }",
        "type Source is protocol { type Item; }; fn probe<T: Source>() -> Int { (&unsafe T.Item).size }",
        "type Source is protocol { type Item; }; type Cell is (); implement Source for Cell { type Item = Int; fn probe(&self) -> Int { Self.Item.size } }",
        "type Source is protocol { type Item; }; type Cell is (); implement Source for Cell { type Item = Int; fn probe(&self) -> Int { (&unsafe Self.Item).size } }",
        "type Source is protocol { type Item; }; type Cell is (); implement Source for Cell { type Item = Int; } fn probe() -> Int { Cell.Item.size }",
    ] {
        let problems = module_errors(TypeChecker::new(), source, false);
        assert!(problems.is_empty(), "{source}: {problems:?}");
    }
    for source in [
        "fn probe<T>() -> Int { (&unsafe T.Missing).size }",
        "type Source is protocol { type Item; }; fn probe<T: Source>() -> Int { (&unsafe T.Missing).size }",
    ] {
        assert!(
            !module_errors(TypeChecker::new(), source, false).is_empty(),
            "unknown projection: {source}"
        );
    }
}

#[test]
fn imported_type_property_does_not_adopt_a_sibling_alias_target() {
    use verum_types::core_metadata::TypeDescriptorKind;
    for reverse in [false, true] {
        let mut metadata = property_metadata();
        metadata
            .types
            .get_mut(&Text::from("core.alpha.Cell"))
            .unwrap()
            .kind = TypeDescriptorKind::Alias {
            target: "Int".into(),
        };
        metadata
            .types
            .get_mut(&Text::from("core.beta.Cell"))
            .unwrap()
            .kind = TypeDescriptorKind::Alias {
            target: "Bool".into(),
        };
        metadata.type_declaration_order = if reverse {
            List::from_iter(["core.beta.Cell".into(), "core.alpha.Cell".into()])
        } else {
            List::from_iter(["core.alpha.Cell".into(), "core.beta.Cell".into()])
        };
        let metadata = std::sync::Arc::new(metadata);
        for eager in [false, true] {
            let checker = || {
                if eager {
                    TypeChecker::new_with_core_eager(metadata.clone())
                } else {
                    TypeChecker::new_with_core(metadata.clone())
                }
            };
            let source = "mount core.alpha as short; fn probe() -> Int { short.Cell.min }";
            let problems = module_errors(checker(), source, true);
            assert!(
                problems.is_empty(),
                "eager={eager} reverse={reverse}: {problems:?}"
            );
            let source = "mount core.beta as short; fn probe() -> Int { short.Cell.min }";
            assert!(
                !module_errors(checker(), source, true).is_empty(),
                "foreign Bool is not Int's numeric property owner"
            );
            let source = "mount core.alpha as short; type Value is { size: Bool }; type Root is { Cell: Value }; fn probe(short: Root) -> Bool { short.Cell.size }";
            let problems = module_errors(checker(), source, true);
            assert!(problems.is_empty(), "local root value: {problems:?}");
        }
    }
}

#[test]
fn type_parameter_mirror_scope_tracks_binding_role_not_type_equality() {
    use verum_types::context::{TypeEnv, TypeScheme};
    use verum_types::ty::{Type, TypeVar};
    let scheme = TypeScheme::mono(Type::Var(TypeVar::fresh()));
    let mut env = TypeEnv::new();
    env.push_scope();
    env.insert_type_parameter_mirror("T", scheme.clone());
    assert!(env.is_locally_bound("T"));
    assert!(env.is_type_parameter_mirror("T"));
    assert!(!env.is_locally_bound_value("T"));
    env.push_scope();
    env.insert("T", scheme.clone());
    assert!(env.is_locally_bound_value("T"));
    assert!(!env.is_type_parameter_mirror("T"));
    assert!(env.remove("T"));
    assert!(env.is_type_parameter_mirror("T"));
    assert!(!env.is_locally_bound_value("T"));
    env.insert("T", scheme.clone());
    env.pop_scope();
    assert!(env.is_type_parameter_mirror("T"));
    env.insert("T", scheme.clone());
    assert!(env.is_locally_bound_value("T"));
    assert!(!env.is_type_parameter_mirror("T"));
    env.insert_type_parameter_mirror("T", scheme.clone());
    assert!(env.remove("T"));
    env.insert("T", scheme.clone());
    assert!(!env.is_type_parameter_mirror("T"));
    env.push_scope();
    env.insert_type_parameter_mirror("T", scheme.clone());
    assert!(!env.is_locally_bound_value("T"));
    env.pop_scope();
    assert!(env.is_locally_bound_value("T"));
    let mut root = TypeEnv::new();
    root.insert_type_parameter_mirror("T", scheme.clone());
    let mut child = root.child();
    child.insert_root("T", scheme);
    assert!(!child.is_type_parameter_mirror("T"));
    child.pop_scope();
    assert!(!child.is_type_parameter_mirror("T"));
    assert!(root.is_type_parameter_mirror("T"));
}

#[test]
fn same_typed_lexical_value_shadows_generic_type_parameter() {
    for source in [
        "type Source is protocol { type Item; }; fn probe<T: Source>(T: T) -> Int { (&unsafe T.Item).size }",
        "type Source is protocol { type Item; }; fn probe<T: Source>(arg: T) -> Int { let T = arg; (&unsafe T.Item).size }",
    ] {
        let problems = module_errors(TypeChecker::new(), source, false);
        assert!(
            problems
                .iter()
                .any(|error| error.contains("unsafe reference requires unsafe block")),
            "{source}: {problems:?}"
        );
    }
}

#[test]
fn value_self_fields_keep_their_declared_type() {
    for property in ["size", "name", "bits", "id", "is_signed"] {
        for operand in [format!("self.{property}"), format!("(self).{property}"), format!("self.inner.{property}")] {
            let source = format!("type Payload is {{ {property}: Bool }}; type Cell is {{ {property}: Bool, inner: Payload }}; implement Cell {{ fn probe(&self)->Bool {{ {operand} }} }}");
            let problems = module_errors(TypeChecker::new(), &source, false);
            assert!(problems.is_empty(), "{source}: {problems:?}");
        }
    }
}
