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

fn assert_registered_owner(checker: &TypeChecker, owner: &str) {
    let protocol = path("Source");
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
                .find_impl(&named(leaf), &path("Source"))
                .map(|entry| format!("{}", entry.for_type)),
            registry
                .find_impl(&named(owner), &path("Source"))
                .map(|entry| format!("{}", entry.for_type))
        );
    }
    assert_registered_owner(&checker, "demo.source.RegistrySource");
    assert_registered_owner(&checker, "demo.source.GitSource");
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
    let consumer = r#"
        mount demo.source.{Source, RegistrySource};
        mount demo.foreign.{RegistrySource as ForeignSource};
        fn locate<S: Source>(source: &S) -> Int { source.value() }
        fn reject(source: &ForeignSource) -> Int { locate(source) }
    "#;
    let (checker, errors) = check(
        consumer,
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
            .find_impl(&named("demo.foreign.RegistrySource"), &path("Source"))
            .is_none()
    );
    assert_eq!(
        errors.len(),
        1,
        "expected only the foreign bound refusal: {errors:?}"
    );
    assert!(
        errors[0].contains("E405") && errors[0].contains("demo.foreign.RegistrySource"),
        "{errors:?}"
    );
}

#[test]
fn local_source_impl_keeps_its_owner_before_body_checking() {
    let source = format!(
        "{PROVIDER} fn locate<S: Source>(source: &S) -> Int {{ source.value() }} fn probe(source: &RegistrySource) -> Int {{ locate(source) }}"
    );
    let (checker, errors) = check(&source, &[]);
    assert!(errors.is_empty(), "{errors:?}");
    assert_registered_owner(&checker, "demo.main.RegistrySource");
    assert_registered_owner(&checker, "demo.main.GitSource");
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
                "public type RegistrySource is { revision: Int };",
            ),
            ("demo.adapter", adapter),
        ],
    );
    assert!(errors.is_empty(), "{errors:?}");
    assert_registered_owner(&checker, "demo.model.RegistrySource");
    let table = checker.protocol_checker.read();
    for false_owner in [
        "Upstream",
        "Mounted",
        "demo.adapter.Upstream",
        "demo.main.Mounted",
    ] {
        assert!(
            table
                .find_impl(&named(false_owner), &path("Source"))
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
                "public type RegistrySource is { revision: Int };",
            )],
        );
        assert!(errors.is_empty(), "{target}: {errors:?}");
        assert_registered_owner(&checker, "demo.model.RegistrySource");
        assert!(
            checker
                .protocol_checker
                .read()
                .find_impl(&named("demo.main.Mounted"), &path("Source"))
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
        .find_impl_with_substitution(&target, &path("Source"))
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
    assert!(table.implements_instantiation(&target, &path("Source"), &[Type::Int]));
    assert!(!table.implements_instantiation(&target, &path("Source"), &[Type::Bool]));
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
        .find_impl(&Type::Int, &path("Source"))
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
        .find_impl(&named("demo.source.RegistrySource"), &path("Source"))
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
