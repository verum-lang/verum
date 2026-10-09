//! T1212: explicit source mounts preserve nominal ownership across homonyms.
use std::sync::Arc;
use verum_ast::{FileId, ItemKind};
use verum_common::{List, Maybe, ResourceDiscipline};
use verum_fast_parser::Parser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::core_metadata::{
    CoreMetadata, FieldDescriptor, TypeDescriptor, TypeDescriptorKind,
};
use verum_types::{Type, TypeChecker};

const DECLARATION: &str = r#"
    public type Cache is { value: Int };
    implement Cache { public fn empty() -> Cache { Cache { value: 37 } } }
"#;

fn foreign_metadata() -> Arc<CoreMetadata> {
    let mut metadata = CoreMetadata::default();
    let descriptor = TypeDescriptor {
        name: "Cache".into(),
        module_path: "core.foreign".into(),
        origin_module_path: Maybe::None,
        generic_params: List::new(),
        kind: TypeDescriptorKind::Record {
            fields: [FieldDescriptor {
                name: "foreign_only".into(),
                ty: "Bool".into(),
                is_public: true,
            }]
            .into_iter()
            .collect(),
        },
        size: Maybe::Some(8),
        alignment: Maybe::Some(8),
        methods: List::new(),
        implements: List::new(),
        decl_span: Maybe::None,
        is_public: true,
        is_transparent_wrapper: false,
        resource_discipline: ResourceDiscipline::Unrestricted,
    };
    metadata.types.insert("Cache".into(), descriptor.clone());
    metadata
        .types
        .insert("core.foreign.Cache".into(), descriptor);
    metadata.type_declaration_order.push("Cache".into());
    Arc::new(metadata)
}

fn check(
    consumer: &str,
    modules: &[(&str, &str)],
    metadata: bool,
    eager: bool,
) -> (TypeChecker, Vec<String>) {
    let mut checker = if metadata {
        if eager {
            TypeChecker::new_with_core_eager(foreign_metadata())
        } else {
            TypeChecker::new_with_core(foreign_metadata())
        }
    } else {
        TypeChecker::new()
    };
    checker.register_primitives();
    checker.set_current_module_path("demo.main");
    let mut registry = ModuleRegistry::new();
    for (index, (owner, source)) in modules
        .iter()
        .chain(std::iter::once(&("demo.main", consumer)))
        .enumerate()
    {
        let ast = Parser::new(source).parse_module().expect("source grammar");
        let id = ModuleId::new(index as u32);
        let path = ModulePath::from_str(owner);
        let mut module = ModuleInfo::new(
            id,
            path.clone(),
            ast.clone(),
            FileId::new(index as u32),
            (*source).into(),
        );
        module.exports = extract_exports_from_module(&ast, id, &path).expect("source exports");
        registry.register(module);
    }
    checker.set_module_registry_direct(registry.clone());
    let ast = Parser::new(consumer)
        .parse_module()
        .expect("consumer grammar");
    // Match the ordinary project pass: metadata preloading precedes mounts.
    checker.register_stdlib_types_for_module(&ast);
    let mut errors = Vec::new();
    for item in &ast.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, "demo.main", &registry) {
                errors.push(format!("{error:?}"));
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
            errors.push(format!("{error:?}"));
        }
    }
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}"));
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| format!("{error:?}")),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| format!("{error:?}")),
    );
    (checker, errors)
}

fn assert_return_owner(checker: &mut TypeChecker, function: &str, expected: &str) {
    let signature = &checker
        .context_mut()
        .env
        .lookup(function)
        .expect("registered function")
        .ty;
    let Type::Function { return_type, .. } = signature else {
        panic!("not a function: {signature:?}")
    };
    let Type::Named { path, .. } = return_type.as_ref() else {
        panic!("not a nominal type: {return_type:?}")
    };
    let actual = path
        .segments
        .iter()
        .map(|segment| match segment {
            verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
            _ => panic!("unexpected nominal owner: {path:?}"),
        })
        .collect::<Vec<_>>()
        .join(".");
    assert_eq!(actual, expected, "{signature:?}");
}

#[test]
fn one_file_declaration_wins_over_public_metadata_homonym() {
    let source = format!(
        "{DECLARATION} fn identity(value: Cache) -> Cache {{ value }} fn probe() -> Int {{ Cache.empty().value }}"
    );
    for eager in [false, true] {
        let (mut checker, errors) = check(&source, &[], true, eager);
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");
        assert_return_owner(&mut checker, "identity", "demo.main.Cache");
    }
}

#[test]
fn explicit_source_mount_keeps_declaration_owner_without_metadata() {
    let (mut checker, errors) = check(
        "mount demo.cache.{Cache}; fn identity(value: Cache) -> Cache { value } fn probe() -> Int { Cache.empty().value }",
        &[("demo.cache", DECLARATION)],
        false,
        false,
    );
    assert!(errors.is_empty(), "{errors:?}");
    assert_return_owner(&mut checker, "identity", "demo.cache.Cache");
    assert!(
        checker.lookup_type_for_testing("demo.main.Cache").is_none(),
        "mount must not declare a consumer-owned type"
    );
}

#[test]
fn explicit_source_mount_beats_public_metadata_homonym() {
    for eager in [false, true] {
        let (mut checker, errors) = check(
            "mount demo.cache.{Cache}; fn identity(value: Cache) -> Cache { value } fn probe() -> Int { Cache.empty().value }",
            &[("demo.cache", DECLARATION)],
            true,
            eager,
        );
        assert!(errors.is_empty(), "eager={eager}: {errors:?}");
        assert_return_owner(&mut checker, "identity", "demo.cache.Cache");
        assert!(
            checker.lookup_type_for_testing("demo.main.Cache").is_none(),
            "mount must not declare a consumer-owned type"
        );
    }
}
