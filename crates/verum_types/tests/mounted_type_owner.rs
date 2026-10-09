//! T1212: explicit source mounts preserve nominal ownership across homonyms.
use std::sync::Arc;
use verum_ast::{FileId, ItemKind};
use verum_common::{List, Maybe, ResourceDiscipline, Text};
use verum_fast_parser::FastParser;
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
) -> (TypeChecker, List<Text>) {
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
        let file_id = FileId::new(index as u32);
        let ast = FastParser::new()
            .parse_module_str(source, file_id)
            .expect("source grammar");
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
    let ast = FastParser::new()
        .parse_module_str(consumer, FileId::new(modules.len() as u32))
        .expect("consumer grammar");
    // Match the ordinary project pass: metadata preloading precedes mounts.
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
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| Text::from(format!("{error:?}"))),
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
        .collect::<List<_>>()
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

#[test]
fn sibling_mount_aliases_keep_both_nominal_owners_in_either_order() {
    let alpha = "public type Cache is { value: Int }; implement Cache { public fn empty() -> Cache { Cache { value: 37 } } public fn read(&self) -> Int { self.value } }";
    let beta = "public type Cache is { value: Bool }; implement Cache { public fn empty() -> Cache { Cache { value: true } } public fn read(&self) -> Bool { self.value } }";
    for reverse in [false, true] {
        let modules = if reverse {
            [("demo.beta", beta), ("demo.alpha", alpha)]
        } else {
            [("demo.alpha", alpha), ("demo.beta", beta)]
        };
        let mounts = if reverse {
            "mount demo.beta.{Cache as B}; mount demo.alpha.{Cache as A};"
        } else {
            "mount demo.alpha.{Cache as A}; mount demo.beta.{Cache as B};"
        };
        let source = format!(
            "{mounts} fn first(value: A) -> demo.alpha.Cache {{ value }} fn second(value: B) -> demo.beta.Cache {{ value }} fn integer() -> Int {{ A.empty().read() }} fn boolean() -> Bool {{ B.empty().read() }}"
        );
        let (mut checker, errors) = check(&source, &modules, true, false);
        assert!(errors.is_empty(), "reverse={reverse}: {errors:?}");
        assert_return_owner(&mut checker, "first", "demo.alpha.Cache");
        assert_return_owner(&mut checker, "second", "demo.beta.Cache");
    }
}

#[test]
fn same_shaped_source_types_do_not_become_the_same_nominal_type() {
    for reverse in [false, true] {
        let mounts = if reverse {
            "mount demo.beta.{Cache as B}; mount demo.alpha.{Cache as A};"
        } else {
            "mount demo.alpha.{Cache as A}; mount demo.beta.{Cache as B};"
        };
        let source = format!("{mounts} fn wrong(value: A) -> B {{ value }}");
        let modules = if reverse {
            [("demo.beta", DECLARATION), ("demo.alpha", DECLARATION)]
        } else {
            [("demo.alpha", DECLARATION), ("demo.beta", DECLARATION)]
        };
        let (checker, errors) = check(&source, &modules, true, false);
        assert!(
            errors.iter().any(|error| error.contains("Mismatch")),
            "reverse={reverse}: {errors:?}; A={:?}; B={:?}; wrong={:?}",
            checker.lookup_type_for_testing("A"),
            checker.lookup_type_for_testing("B"),
            checker.lookup_qualified_name_for_testing("wrong"),
        );
    }
}

#[test]
fn mounted_newtype_keeps_its_owner_and_tuple_projection() {
    let declaration =
        "public type Cache is (Int); implement Cache { public fn empty() -> Cache { Cache(37) } }";
    for metadata in [false, true] {
        let (mut checker, errors) = check(
            "mount demo.cache.{Cache}; fn identity(value: Cache) -> Cache { value } fn probe() -> Int { Cache.empty().0 } fn read(value: &Cache) -> Int { value.0 }",
            &[("demo.cache", declaration)],
            metadata,
            false,
        );
        assert!(errors.is_empty(), "metadata={metadata}: {errors:?}");
        assert_return_owner(&mut checker, "identity", "demo.cache.Cache");
    }
}

#[test]
fn mounted_generic_record_keeps_static_and_instance_api() {
    let declaration = "public type Cache<T> is { value: T }; implement<T> Cache<T> { public fn from_value(value: T) -> Cache<T> { Cache { value } } public fn take(self) -> T { self.value } }";
    let (mut checker, errors) = check(
        "mount demo.cache.{Cache}; fn identity(value: Cache<Int>) -> Cache<Int> { value } fn probe() -> Int { Cache<Int>.from_value(37).take() }",
        &[("demo.cache", declaration)],
        true,
        false,
    );
    assert!(errors.is_empty(), "{errors:?}");
    assert_return_owner(&mut checker, "identity", "demo.cache.Cache");
}

#[test]
fn renamed_reexport_keeps_the_original_declaration_owner() {
    let facade = "public mount demo.cache.{Cache as Published};";
    let (mut checker, errors) = check(
        "mount demo.facade.{Published as Local}; fn identity(value: Local) -> demo.cache.Cache { value } fn probe() -> Int { Local.empty().value }",
        &[("demo.cache", DECLARATION), ("demo.facade", facade)],
        true,
        false,
    );
    assert!(errors.is_empty(), "{errors:?}");
    assert_return_owner(&mut checker, "identity", "demo.cache.Cache");
}

#[test]
fn metadata_homonym_cannot_supply_a_missing_explicit_source_export() {
    for declaration in [
        "public type Other is { value: Int };",
        "type Cache is { value: Int };",
    ] {
        let (_, errors) = check(
            "mount demo.cache.{Cache}; fn identity(value: Cache) -> Cache { value }",
            &[("demo.cache", declaration)],
            true,
            false,
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("ImportItemNotFound")
                    || error.contains("ImportNotFound")
                    || error.contains("Private")),
            "{declaration}: {errors:?}"
        );
    }
}

#[test]
fn mounted_generic_static_method_rejects_the_wrong_explicit_receiver_argument() {
    let declaration = "public type Cache<T> is { value: T }; implement<T> Cache<T> { public fn from_value(value: T) -> Cache<T> { Cache { value } } }";
    let (_, errors) = check(
        "mount demo.cache.{Cache}; fn probe() -> Int { Cache<Bool>.from_value(37).value }",
        &[("demo.cache", declaration)],
        true,
        false,
    );
    assert!(
        errors.iter().any(|error| error.contains("Mismatch")),
        "{errors:?}"
    );
}

#[test]
fn tuple_constructors_and_fields_preserve_both_owners() {
    for reverse in [false, true] {
        let modules = if reverse {
            [
                ("demo.beta", "public type Cache is (Bool, Int);"),
                ("demo.alpha", "public type Cache is (Int, Bool);"),
            ]
        } else {
            [
                ("demo.alpha", "public type Cache is (Int, Bool);"),
                ("demo.beta", "public type Cache is (Bool, Int);"),
            ]
        };
        let mounts = if reverse {
            "mount demo.beta.{Cache as B}; mount demo.alpha.{Cache as A};"
        } else {
            "mount demo.alpha.{Cache as A}; mount demo.beta.{Cache as B};"
        };
        let source = format!(
            "{mounts} fn first() -> demo.alpha.Cache {{ A(37, true) }} fn second() -> demo.beta.Cache {{ B(true, 37) }} fn read_a(value: &A) -> Int {{ value.0 }} fn read_b(value: &B) -> Bool {{ value.0 }}"
        );
        let (_, errors) = check(&source, &modules, true, false);
        assert!(errors.is_empty(), "reverse={reverse}: {errors:?}");
    }
}

#[test]
fn one_file_tuple_constructor_keeps_its_owner_after_both_declaration_passes() {
    for declaration in [
        "public type Cache is (Int);",
        "public type Cache is (Int, Bool);",
    ] {
        let source = format!(
            "{declaration} fn identity(value: Cache) -> Cache {{ value }} fn read(value: &Cache) -> Int {{ value.0 }}"
        );
        let (mut checker, errors) = check(&source, &[], true, false);
        assert!(errors.is_empty(), "{declaration}: {errors:?}");
        assert_return_owner(&mut checker, "identity", "demo.main.Cache");
    }
}
