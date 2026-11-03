//! T1686: source implementation targets retain their declaration owner.
use verum_ast::ty::{Ident, Path, PathSegment};
use verum_ast::{FileId, ItemKind, Span};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::{Type, TypeChecker};

const PROVIDER: &str = r#"
    public type Source is protocol {
        type Reference;
        fn value(&self) -> Int;
    };
    public type RegistrySource is { revision: Int };
    public type GitSource is { revision: Int };
    implement Source for RegistrySource {
        type Reference = Int;
        fn value(&self) -> Int { self.revision }
    }
    implement Source for GitSource {
        type Reference = Int;
        fn value(&self) -> Int { self.revision }
    }
"#;

fn path(name: &str) -> Path {
    Path::new(
        name.split('.')
            .map(|name| PathSegment::Name(Ident::new(name, Span::dummy())))
            .collect(),
        Span::dummy(),
    )
}

fn named(name: &str) -> Type {
    Type::Named {
        path: path(name),
        args: List::new(),
    }
}

fn check(consumer: &str, modules: &[(&str, &str)]) -> (TypeChecker, List<Text>) {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog("demo");
    checker.set_current_module_path("demo.main");
    let mut registry = ModuleRegistry::new();
    for (index, (owner, source)) in modules
        .iter()
        .chain(std::iter::once(&("demo.main", consumer)))
        .enumerate()
    {
        let file_id = FileId::new(index as u32);
        let ast = FastParser::new()
            .parse_module_str(source, file_id)
            .expect("source grammar");
        let id = ModuleId::new(index as u32);
        let path = ModulePath::from_str(owner);
        let mut module = ModuleInfo::new(id, path.clone(), ast.clone(), file_id, (*source).into());
        module.exports = extract_exports_from_module(&ast, id, &path).expect("source exports");
        registry.register(module);
    }
    checker.set_module_registry_direct(registry.clone());
    let ast = FastParser::new()
        .parse_module_str(consumer, FileId::new(modules.len() as u32))
        .expect("consumer grammar");
    checker.register_stdlib_types_for_module(&ast);
    let mut errors = List::new();
    for item in &ast.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, "demo.main", &registry) {
                errors.push(format!("{error:?}").into());
            }
        }
    }
    for item in &ast.items {
        let result = match &item.kind {
            ItemKind::Type(decl) => checker.register_type_declaration(decl),
            ItemKind::Impl(decl) => checker.register_impl_block(decl),
            ItemKind::Function(decl) => checker.register_function_signature(decl),
            _ => continue,
        };
        if let Err(error) = result {
            errors.push(format!("{error:?}").into());
        }
    }
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    (checker, errors)
}

fn assert_registered_owner(checker: &TypeChecker, owner: &str, protocol_owner: &str) {
    let protocol = path(protocol_owner);
    let leaf = owner.rsplit('.').next().expect("owner leaf");
    let registry = checker.protocol_checker.read();
    let bare = registry.find_impl(&named(leaf), &protocol);
    let qualified = registry.find_impl(&named(owner), &protocol);
    eprintln!(
        "target table {owner}: bare={:?}; qualified={:?}",
        bare.map(|entry| (&entry.for_type, &entry.protocol)),
        qualified.map(|entry| (&entry.for_type, &entry.protocol))
    );
    let implementation = qualified.expect("source implementation must use the declaration owner");
    assert_eq!(format!("{}", implementation.for_type), owner);
    assert!(
        implementation.methods.contains_key(&Text::from("value")),
        "complete source methods must be retained"
    );
    assert_eq!(
        implementation
            .associated_types
            .get(&Text::from("Reference")),
        Some(&Type::Int)
    );
    assert!(
        bare.is_none(),
        "unqualified name must not be a second nominal owner"
    );
}

fn check_source_mounts(mounts: &str) {
    let consumer = format!(
        r#"
        mount demo.source.{{{mounts}}};
        fn locate<S: Source>(source: &S) -> Int {{ source.value() }}
        fn from_registry(source: &RegistrySource) -> Int {{ locate(source) }}
        fn from_git(source: &GitSource) -> Int {{ locate(source) }}
    "#
    );
    let (checker, errors) = check(&consumer, &[("demo.source", PROVIDER)]);
    eprintln!("bound diagnostics ({mounts}): {errors:?}");
    for owner in ["demo.source.RegistrySource", "demo.source.GitSource"] {
        let registry = checker.protocol_checker.read();
        let leaf = owner.rsplit('.').next().unwrap();
        eprintln!(
            "registration {owner}: bare={:?}, qualified={:?}",
            registry
                .find_impl(&named(leaf), &path("demo.source.Source"))
                .map(|entry| format!("{}", entry.for_type)),
            registry
                .find_impl(&named(owner), &path("demo.source.Source"))
                .map(|entry| format!("{}", entry.for_type))
        );
    }
    assert_registered_owner(&checker, "demo.source.RegistrySource", "demo.source.Source");
    assert_registered_owner(&checker, "demo.source.GitSource", "demo.source.Source");
    assert!(
        errors.is_empty(),
        "bound calls must accept actual source implementations: {errors:?}"
    );
}

#[test]
fn source_impl_targets_keep_declaring_owner() {
    check_source_mounts("Source, RegistrySource, GitSource");
}

#[test]
fn source_impl_targets_do_not_depend_on_mount_order() {
    check_source_mounts("RegistrySource, GitSource, Source");
}

#[test]
fn unrelated_same_name_type_does_not_inherit_source_impl() {
    for reverse in [false, true] {
        let mounts = if reverse {
            "mount demo.foreign.{RegistrySource as ForeignSource}; mount demo.source.{Source, RegistrySource};"
        } else {
            "mount demo.source.{Source, RegistrySource}; mount demo.foreign.{RegistrySource as ForeignSource};"
        };
        let consumer = format!(
            r#"
            {mounts}
            fn locate<S: Source>(source: &S) -> Int {{ source.value() }}
            fn reject(source: &ForeignSource) -> Int {{ locate(source) }}
        "#
        );
        let (checker, errors) = check(
            &consumer,
            &[
                ("demo.source", PROVIDER),
                (
                    "demo.foreign",
                    "public type RegistrySource is { revision: Int };",
                ),
            ],
        );
        assert!(
            checker
                .protocol_checker
                .read()
                .find_impl(
                    &named("demo.foreign.RegistrySource"),
                    &path("demo.source.Source")
                )
                .is_none()
        );
        assert_eq!(
            errors.len(),
            1,
            "reverse={reverse}: expected only the foreign bound refusal: {errors:?}"
        );
        assert!(
            errors[0].contains("E405") && errors[0].contains("demo.foreign.RegistrySource"),
            "reverse={reverse}: {errors:?}"
        );
    }
}

#[test]
fn early_source_impl_loading_does_not_reroute_an_explicit_foreign_binding() {
    for reverse in [false, true] {
        let mounts = if reverse {
            "mount demo.foreign.{RegistrySource}; mount demo.source.{Source};"
        } else {
            "mount demo.source.{Source}; mount demo.foreign.{RegistrySource};"
        };
        let consumer = format!(
            r#"
            {mounts}
            fn locate<S: Source>(source: &S) -> Int {{ source.value() }}
            fn reject(source: &RegistrySource) -> Int {{ locate(source) }}
        "#
        );
        let (checker, errors) = check(
            &consumer,
            &[
                ("demo.source", PROVIDER),
                (
                    "demo.foreign",
                    "public type RegistrySource is { revision: Int };",
                ),
            ],
        );
        assert_registered_owner(&checker, "demo.source.RegistrySource", "demo.source.Source");
        assert!(
            checker
                .protocol_checker
                .read()
                .find_impl(
                    &named("demo.foreign.RegistrySource"),
                    &path("demo.source.Source")
                )
                .is_none()
        );
        assert_eq!(errors.len(), 1, "reverse={reverse}: {errors:?}");
        assert!(
            errors[0].contains("E405") && errors[0].contains("demo.foreign.RegistrySource"),
            "source prepass changed the explicit receiver binding: reverse={reverse}: {errors:?}"
        );
    }
}

#[test]
fn local_source_impl_keeps_its_owner_before_body_checking() {
    let source = format!(
        "{PROVIDER} fn locate<S: Source>(source: &S) -> Int {{ source.value() }} fn probe(source: &RegistrySource) -> Int {{ locate(source) }}"
    );
    let (checker, errors) = check(&source, &[]);
    assert!(errors.is_empty(), "{errors:?}");
    assert_registered_owner(&checker, "demo.main.RegistrySource", "demo.main.Source");
    assert_registered_owner(&checker, "demo.main.GitSource", "demo.main.Source");
}

#[test]
fn renamed_reexported_target_retains_its_original_declaration_owner() {
    let adapter = r#"
        public mount demo.model.{RegistrySource as Upstream};
        public type Source is protocol { type Reference; fn value(&self) -> Int; };
        implement Source for Upstream {
            type Reference = Int;
            fn value(&self) -> Int { self.revision }
        }
    "#;
    let consumer = r#"
        mount demo.adapter.{Source, Upstream as Mounted};
        fn locate<S: Source>(source: &S) -> Int { source.value() }
        fn probe(source: &Mounted) -> Int { locate(source) }
    "#;
    let (checker, errors) = check(
        consumer,
        &[
            (
                "demo.model",
                "public type RegistrySource is { public revision: Int };",
            ),
            ("demo.adapter", adapter),
        ],
    );
    assert!(errors.is_empty(), "{errors:?}");
    assert_registered_owner(&checker, "demo.model.RegistrySource", "demo.adapter.Source");
    let table = checker.protocol_checker.read();
    for false_owner in [
        "Upstream",
        "Mounted",
        "demo.adapter.Upstream",
        "demo.main.Mounted",
    ] {
        assert!(
            table
                .find_impl(&named(false_owner), &path("demo.adapter.Source"))
                .is_none(),
            "invented owner {false_owner}"
        );
    }
}

#[test]
fn local_impl_for_imported_alias_or_qualified_target_keeps_foreign_owner() {
    for target in ["Mounted", "demo.model.RegistrySource"] {
        let source = format!(
            r#"
            mount demo.model.{{RegistrySource as Mounted}};
            public type Source is protocol {{ type Reference; fn value(&self) -> Int; }};
            fn locate<S: Source>(source: &S) -> Int {{ source.value() }}
            fn probe(source: &Mounted) -> Int {{ locate(source) }}
            implement Source for {target} {{
                type Reference = Int;
                fn value(&self) -> Int {{ self.revision }}
            }}
        "#
        );
        let (checker, errors) = check(
            &source,
            &[(
                "demo.model",
                "public type RegistrySource is { public revision: Int };",
            )],
        );
        assert!(errors.is_empty(), "{target}: {errors:?}");
        assert_registered_owner(&checker, "demo.model.RegistrySource", "demo.main.Source");
        assert!(
            checker
                .protocol_checker
                .read()
                .find_impl(&named("demo.main.Mounted"), &path("demo.main.Source"))
                .is_none()
        );
    }
}

#[test]
fn generic_source_target_keeps_reordered_parameter_bindings() {
    let provider = r#"
        public type Source is protocol { type Reference; fn value(&self) -> Int; };
        public type Pair<First, Second> is { first: First, second: Second };
        implement<Left, Right> Source for Pair<Right, Left> {
            type Reference = Left;
            fn value(&self) -> Int { 7 }
        }
    "#;
    let consumer = r#"
        mount demo.source.{Source, Pair};
        fn locate<S: Source>(source: &S) -> Int { source.value() }
        fn probe(source: &Pair<Int, Bool>) -> Int { locate(source) }
    "#;
    let (checker, errors) = check(consumer, &[("demo.source", provider)]);
    assert!(errors.is_empty(), "{errors:?}");
    let target = Type::Named {
        path: path("demo.source.Pair"),
        args: [Type::Int, Type::Bool].into_iter().collect(),
    };
    let table = checker.protocol_checker.read();
    let (implementation, substitution) = table
        .find_impl_with_substitution(&target, &path("demo.source.Source"))
        .expect("generic implementation");
    let Type::Named { args, .. } = &implementation.for_type else {
        panic!("nominal target required")
    };
    let [Type::Var(right), Type::Var(left)] = args.as_slice() else {
        panic!("declared parameter variables required: {args:?}")
    };
    assert_ne!(left, right);
    assert_eq!(
        substitution.get(&Text::from(format!("T{}", right.id()))),
        Some(&Type::Int)
    );
    assert_eq!(
        substitution.get(&Text::from(format!("T{}", left.id()))),
        Some(&Type::Bool)
    );
    assert_eq!(
        implementation
            .associated_types
            .get(&Text::from("Reference")),
        Some(&Type::Var(*left))
    );
    assert!(implementation.methods.contains_key(&Text::from("value")));
}

#[test]
fn source_protocol_argument_identity_is_preserved() {
    let provider = r#"
        public type Source<Argument> is protocol { fn value(&self) -> Int; };
        public type RegistrySource is { revision: Int };
        implement Source<Int> for RegistrySource { fn value(&self) -> Int { self.revision } }
    "#;
    let (checker, errors) = check(
        "mount demo.source.{Source, RegistrySource};",
        &[("demo.source", provider)],
    );
    assert!(errors.is_empty(), "{errors:?}");
    let table = checker.protocol_checker.read();
    let target = named("demo.source.RegistrySource");
    assert!(table.implements_instantiation(&target, &path("demo.source.Source"), &[Type::Int]));
    assert!(!table.implements_instantiation(&target, &path("demo.source.Source"), &[Type::Bool]));
}

#[test]
fn genuine_blanket_target_remains_a_type_parameter() {
    let provider = r#"
        public type Source is protocol { fn value(&self) -> Int; };
        implement<ElementType> Source for ElementType { fn value(&self) -> Int { 7 } }
    "#;
    let (checker, errors) = check(
        "mount demo.source.{Source}; fn probe<S: Source>(source: &S) -> Int { source.value() } fn integer(value: &Int) -> Int { probe(value) }",
        &[("demo.source", provider)],
    );
    assert!(errors.is_empty(), "{errors:?}");
    let table = checker.protocol_checker.read();
    let implementation = table
        .find_impl(&Type::Int, &path("demo.source.Source"))
        .expect("blanket applies to Int");
    assert!(matches!(implementation.for_type, Type::Var(_)));
}

#[test]
fn source_registration_does_not_relax_coherence_for_another_declaration() {
    let (checker, errors) = check(
        "mount demo.source.{Source, RegistrySource};",
        &[("demo.source", PROVIDER)],
    );
    assert!(errors.is_empty(), "{errors:?}");
    let mut table = checker.protocol_checker.write();
    let mut other = table
        .find_impl(
            &named("demo.source.RegistrySource"),
            &path("demo.source.Source"),
        )
        .expect("actual imported implementation")
        .clone();
    other.span = Span::new(other.span.start + 1, other.span.end + 1, other.span.file_id);
    let before = table.all_implementations().len();
    assert!(matches!(
        table.register_impl(other),
        Err(verum_types::protocol::CoherenceError::OverlappingImplementations { .. })
    ));
    assert_eq!(table.all_implementations().len(), before);
}

#[test]
fn dyn_receiver_access_survives_a_deref_and_protocol_inheritance() {
    let declarations = r#"
        type Parent is protocol { fn inherited(&self) -> Int; };
        type Source is protocol extends Parent {
            fn read(&self) -> Int;
            fn change(&mut self) -> Int;
        };
        type Deref is protocol { type Target; fn deref(&self) -> &Self.Target; };
        type Holder<T> is { inner: T };
        implement<T> Deref for Holder<T> {
            type Target = T;
            fn deref(&self) -> &T { &self.inner }
        }
    "#;
    for receiver in ["&Holder<dyn Source>", "&(dyn Source)"] {
        for (method, writable) in [("read", false), ("inherited", false), ("change", true)] {
            let source =
                format!("{declarations} fn probe(x: {receiver}) -> Int {{ x.{method}() }}");
            let (_, errors) = check(&source, &[]);
            if writable {
                assert!(
                    errors.iter().any(|e| e.contains("SharedReferenceWrite")),
                    "{receiver} {method}: {errors:?}"
                );
            } else {
                assert!(errors.is_empty(), "{receiver} {method}: {errors:?}");
            }
        }
    }
}

#[test]
fn dyn_calls_check_arguments_against_the_selected_protocol_method() {
    let declarations = r#"
        type Parent is protocol { fn inherited(&self, value: Int) -> Int; };
        type Source is protocol extends Parent { fn read(&self, value: Int) -> Int; };
    "#;
    for method in ["read", "inherited"] {
        for (arguments, diagnostic) in [
            ("1", None),
            ("true", Some("Mismatch")),
            ("", Some("WrongArgCount")),
            ("1, 2", Some("WrongArgCount")),
        ] {
            let source = format!(
                "{declarations} fn probe(x: &(dyn Source)) -> Int {{ x.{method}({arguments}) }}"
            );
            let (_, errors) = check(&source, &[]);
            let matches = match diagnostic {
                None => errors.is_empty(),
                Some(kind) => errors.iter().any(|error| error.contains(kind)),
            };
            assert!(
                matches,
                "x.{method}({arguments}): expected {diagnostic:?}, got {errors:?}"
            );
        }
    }
}

#[test]
fn inherited_methods_keep_distinct_same_leaf_protocol_owners() {
    let modules = [
        (
            "demo.alpha",
            "public type Parent is protocol { fn alpha(&self, value: Int) -> Int; };",
        ),
        (
            "demo.beta",
            "public type Parent is protocol { fn beta(&self, value: Bool) -> Int; };",
        ),
    ];
    for parents in ["Alpha + Beta", "Beta + Alpha"] {
        for (method, valid_argument, invalid_argument) in
            [("alpha", "1", "true"), ("beta", "true", "1")]
        {
            for (argument, valid) in [(valid_argument, true), (invalid_argument, false)] {
                let source = format!(
                    r#"
                    mount demo.alpha.{{Parent as Alpha}};
                    mount demo.beta.{{Parent as Beta}};
                    type Combined is protocol extends {parents} {{}};
                    fn probe(value: &(dyn Combined)) -> Int {{ value.{method}({argument}) }}
                "#
                );
                let (_, errors) = check(&source, &modules);
                if valid {
                    assert!(
                        errors.is_empty(),
                        "{parents}: {method}({argument}): {errors:?}"
                    );
                } else {
                    assert!(
                        errors.iter().any(|error| error.contains("Mismatch")),
                        "{parents}: {method}({argument}) must use its owned signature: {errors:?}"
                    );
                }
            }
        }
    }
}
