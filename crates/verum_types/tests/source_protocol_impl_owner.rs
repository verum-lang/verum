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
