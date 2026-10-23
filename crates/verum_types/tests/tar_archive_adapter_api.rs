//! T1741: check the real tar adapter across its declaration boundary.
//!
//! All selected declarations, mounts, signatures and tested bodies come from
//! current source. Reader/writer dependency bodies and runtime are not tested.
use verum_ast::{
    FileId, Item, ItemKind, Module, Visibility,
    decl::{ImplItemKind, ImplKind, TypeDeclBody},
    ty::TypeKind,
};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::TypeChecker;

const TAR: &str = "core.archive.tar";
const ARCHIVE: &str = "core.archive";
const TAR_SOURCE: &str = include_str!("../../../core/archive/tar.vr");
const ARCHIVE_SOURCE: &str = include_str!("../../../core/archive/mod.vr");
const CONSTRUCTORS: &[&str] = &[
    "entry_for_file",
    "entry_for_directory",
    "entry_for_symlink",
    "entry_for_symlink_with_mode",
    "entry_for_hardlink",
];
const IMPORTS: &str = "mount core.base.maybe.Maybe; mount core.base.result.Result; mount core.collections.list.List; mount core.archive.tar.{TarEntry, EntryType}; mount core.archive.{ArchiveAlgorithm, ArchiveEntryKind, ArchiveEntry, ArchiveError, Archive, TarFormat};";

fn parse(source: &str, file: u32) -> Module {
    FastParser::new()
        .parse_module_str(source, FileId::new(file))
        .expect("actual source or control grammar")
}

fn impl_target(item: &Item, name: &str) -> bool {
    let ItemKind::Impl(implementation) = &item.kind else {
        return false;
    };
    let target = match &implementation.kind {
        ImplKind::Inherent(target) => target,
        ImplKind::Protocol { for_type, .. } => for_type,
    };
    let target = match &target.kind {
        TypeKind::Generic { base, .. } => base.as_ref(),
        _ => target,
    };
    matches!(&target.kind, TypeKind::Path(path)
        if path.as_ident().is_some_and(|ident| ident.name == name))
        || matches!(&target.kind, TypeKind::Text if name == "Text")
}

fn modules() -> List<(&'static str, &'static str, Module)> {
    let inputs = [
        (
            "core.base",
            include_str!("../../../core/base/mod.vr"),
            &[][..],
        ),
        (
            "core.base.maybe",
            include_str!("../../../core/base/maybe.vr"),
            &["Maybe"][..],
        ),
        (
            "core.base.result",
            include_str!("../../../core/base/result.vr"),
            &["Result"][..],
        ),
        (
            "core.collections",
            include_str!("../../../core/collections/mod.vr"),
            &[][..],
        ),
        (
            "core.collections.list",
            include_str!("../../../core/collections/list.vr"),
            &["List"][..],
        ),
        (
            "core.text.text",
            include_str!("../../../core/text/text.vr"),
            &["Text"][..],
        ),
        (TAR, TAR_SOURCE, &["EntryType", "TarEntry", "TarError"][..]),
        (
            ARCHIVE,
            ARCHIVE_SOURCE,
            &[
                "ArchiveEntry",
                "ArchiveEntryKind",
                "ArchiveError",
                "ArchiveAlgorithm",
                "Archive",
                "TarFormat",
            ][..],
        ),
    ];
    inputs
        .into_iter()
        .enumerate()
        .map(|(index, (owner, source, types))| {
            let mut ast = parse(source, index as u32);
            if !types.is_empty() {
                ast.items.retain_mut(|item| {
                    if owner == "core.text.text" && impl_target(item, "Text") {
                        let ItemKind::Impl(implementation) = &mut item.kind else { unreachable!() };
                        implementation.items.retain(|member| matches!(&member.kind, ImplItemKind::Function(function) if matches!(function.name.name.as_str(), "to_text" | "clone")));
                        return !implementation.items.is_empty();
                    }
                    if (owner == "core.collections.list" && impl_target(item, "List")) || (owner == "core.base.maybe" && impl_target(item, "Maybe")) {
                        let ItemKind::Impl(implementation) = &mut item.kind else { unreachable!() };
                        implementation.items.retain(|member| matches!(&member.kind, ImplItemKind::Function(function) if matches!(function.name.name.as_str(), "new" | "len" | "get" | "push" | "clone" | "unwrap")));
                        return !implementation.items.is_empty();
                    }
                    match &item.kind {
                    ItemKind::Mount(_) => true,
                    ItemKind::Type(declaration) => types.contains(&declaration.name.name.as_str()),
                    ItemKind::Impl(_) => {
                        (owner == TAR && impl_target(item, "TarEntry"))
                            || (owner == ARCHIVE && impl_target(item, "TarFormat"))
                    }
                    ItemKind::Function(function) => {
                        (owner == TAR
                            && (CONSTRUCTORS.contains(&function.name.name.as_str())
                                || matches!(
                                    function.name.name.as_str(),
                                    "read_archive" | "write_archive"
                                )))
                            || (owner == ARCHIVE
                                && matches!(
                                    function.name.name.as_str(),
                                    "tar_entry_to_universal"
                                        | "universal_to_tar_entry"
                                        | "tar_format"
                                ))
                    }
                    _ => false,
                    }
                });
            }
            (owner, source, ast)
        })
        .collect()
}

fn diagnostics(checker: &mut TypeChecker) -> List<Text> {
    let mut errors: List<Text> = checker
        .take_deferred_errors()
        .into_iter()
        .map(|error| format!("{error:?}").into())
        .collect();
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| format!("{error:?}").into()),
    );
    errors
}

fn checker(owner: &str) -> (TypeChecker, ModuleRegistry) {
    let mut registry = ModuleRegistry::new();
    for (index, (path, source, ast)) in modules().into_iter().enumerate() {
        let id = ModuleId::new(index as u32);
        let path = ModulePath::from_str(path);
        let mut info = ModuleInfo::new(
            id,
            path.clone(),
            ast.clone(),
            FileId::new(index as u32),
            source.into(),
        );
        info.exports = extract_exports_from_module(&ast, id, &path).expect("actual exports");
        registry.register(info);
    }
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog(if owner.starts_with("core.") {
        "core"
    } else {
        "consumer"
    });
    checker.set_current_module_path(owner);
    checker.set_module_registry_direct(registry.clone());
    // Text is a primitive receiver, so its actual source implementation is
    // installed explicitly rather than pretending a record import loads it.
    checker.set_current_module_path("core.text.text");
    let text_module = modules()
        .into_iter()
        .find(|(path, _, _)| *path == "core.text.text")
        .unwrap()
        .2;
    for item in &text_module.items {
        if let ItemKind::Impl(implementation) = &item.kind {
            checker
                .register_impl_block(implementation)
                .expect("actual Text API interface");
        }
    }
    checker.set_current_module_path(owner);
    for item in &parse(IMPORTS, 80).items {
        if let ItemKind::Mount(import) = &item.kind {
            checker
                .process_import(import, owner, &registry)
                .expect("public dependency imports");
        }
    }
    // This is the original parent re-export, including its exact function aliases.
    let archive = parse(ARCHIVE_SOURCE, 81);
    for item in &archive.items {
        if let ItemKind::Mount(import) = &item.kind {
            if import.visibility == Visibility::Public {
                checker
                    .process_import(import, ARCHIVE, &registry)
                    .expect("actual tar API re-export");
            }
        }
    }
    for item in &archive.items {
        if let ItemKind::Function(function) = &item.kind {
            if matches!(
                function.name.name.as_str(),
                "tar_entry_to_universal" | "universal_to_tar_entry" | "tar_format"
            ) {
                checker.set_current_module_path(ARCHIVE);
                checker
                    .register_function_signature(function)
                    .expect("actual adapter signature");
            }
        }
    }
    checker.set_current_module_path(TAR);
    let tar_module = modules()
        .into_iter()
        .find(|(path, _, _)| *path == TAR)
        .unwrap()
        .2;
    for item in &tar_module.items {
        if let ItemKind::Function(function) = &item.kind {
            checker
                .register_function_signature(function)
                .expect("actual tar function interface");
        }
    }
    checker.set_current_module_path(owner);
    let errors = diagnostics(&mut checker);
    assert!(
        errors.is_empty(),
        "dependency setup must precede body oracles: {errors:?}"
    );
    (checker, registry)
}

fn check(owner: &str, items: &[Item]) -> List<Text> {
    let (mut checker, registry) = checker(owner);
    let mut errors = List::new();
    for item in items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, owner, &registry) {
                errors.push(format!("{error:?}").into());
            }
        }
        if let ItemKind::Function(function) = &item.kind {
            if let Err(error) = checker.register_function_signature(function) {
                errors.push(format!("{error:?}").into());
            }
        }
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(diagnostics(&mut checker));
    errors
}

fn function(source: &str, name: &str) -> Item {
    parse(source, 90)
        .items
        .into_iter()
        .find(
            |item| matches!(&item.kind, ItemKind::Function(function) if function.name.name == name),
        )
        .unwrap_or_else(|| panic!("actual function {name} must exist"))
}

#[test]
fn actual_mounts_and_tar_storage_policies_are_preserved() {
    for (index, (_, source, selected)) in modules().iter().enumerate() {
        let original = parse(source, index as u32);
        let mounts = |ast: &Module| -> List<Item> {
            ast.items
                .iter()
                .filter(|item| matches!(item.kind, ItemKind::Mount(_)))
                .cloned()
                .collect()
        };
        assert_eq!(mounts(selected), mounts(&original));
    }
    let ast = parse(TAR_SOURCE, 91);
    let fields = ast
        .items
        .iter()
        .find_map(|item| match &item.kind {
            ItemKind::Type(declaration) if declaration.name.name == "TarEntry" => {
                match &declaration.body {
                    TypeDeclBody::Record(fields) => Some(fields),
                    _ => None,
                }
            }
            _ => None,
        })
        .expect("actual TarEntry record");
    assert_eq!(fields.len(), 10);
    assert!(
        fields
            .iter()
            .all(|field| field.visibility == Visibility::Private)
    );
}

#[test]
fn actual_parent_forward_adapter_body_checks() {
    let errors = check(
        ARCHIVE,
        &[function(ARCHIVE_SOURCE, "tar_entry_to_universal")],
    );
    assert!(errors.is_empty(), "actual forward adapter body: {errors:?}");
}

#[test]
fn actual_parent_reverse_adapter_body_checks() {
    let errors = check(
        ARCHIVE,
        &[function(ARCHIVE_SOURCE, "universal_to_tar_entry")],
    );
    assert!(errors.is_empty(), "actual reverse adapter body: {errors:?}");
}

#[test]
fn actual_tar_format_methods_check_with_fallible_conversion() {
    let item = parse(ARCHIVE_SOURCE, 92)
        .items
        .into_iter()
        .find(|item| impl_target(item, "TarFormat"))
        .expect("actual Archive implementation");
    let errors = check(ARCHIVE, &[item]);
    assert!(errors.is_empty(), "actual TarFormat methods: {errors:?}");
}

#[test]
fn metadata_accessors_are_public_scalar_copies_with_real_bodies() {
    let ast = parse(TAR_SOURCE, 93);
    let implementation = ast
        .items
        .iter()
        .find(|item| impl_target(item, "TarEntry"))
        .expect("actual TarEntry implementation");
    let ItemKind::Impl(declaration) = &implementation.kind else {
        unreachable!()
    };
    for name in ["mode", "mtime"] {
        let (item, function) = declaration
            .items
            .iter()
            .find_map(|item| match &item.kind {
                ImplItemKind::Function(function) if function.name.name == name => {
                    Some((item, function))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("public metadata accessor {name} must exist"));
        assert_eq!(item.visibility, Visibility::Public);
        assert!(function.body.is_some());
        assert!(matches!(
            function.return_type.as_ref().map(|ty| &ty.kind),
            Some(TypeKind::Int)
        ));
    }
    let errors = check(TAR, &[implementation.clone()]);
    assert!(errors.is_empty(), "actual owner getter bodies: {errors:?}");
}

#[test]
fn actual_link_constructors_check_inside_the_declaring_owner() {
    for name in [
        "entry_for_hardlink",
        "entry_for_symlink_with_mode",
        "entry_for_symlink",
    ] {
        let item = function(TAR_SOURCE, name);
        let ItemKind::Function(declaration) = &item.kind else {
            unreachable!()
        };
        assert_eq!(declaration.visibility, Visibility::Public);
        let errors = check(TAR, &[item]);
        assert!(errors.is_empty(), "actual {name} body: {errors:?}");
    }
}

#[test]
fn foreign_callers_receive_an_explicit_conversion_result() {
    let item = function(ARCHIVE_SOURCE, "tar_entry_to_universal");
    let ItemKind::Function(declaration) = &item.kind else {
        unreachable!()
    };
    let Some(result) = &declaration.return_type else {
        panic!("explicit result type")
    };
    assert!(
        matches!(&result.kind, TypeKind::Generic { base, args }
        if matches!(&base.kind, TypeKind::Path(path) if path.as_ident().is_some_and(|name| name.name == "Result"))
            && args.len() == 2),
        "the public declaration must expose Result, not an implicit value coercion"
    );
    let probe = parse(
        "mount core.archive.tar_entry_to_universal; fn convert(entry: &TarEntry) -> Result<ArchiveEntry, ArchiveError> { tar_entry_to_universal(entry) }",
        94,
    );
    let errors = check("consumer.main", &probe.items);
    assert!(errors.is_empty(), "fallible public converter: {errors:?}");
}

#[test]
fn foreign_metadata_and_link_constructor_calls_keep_exact_types() {
    let probe = parse(
        "mount core.archive.tar.{entry_for_hardlink, entry_for_symlink_with_mode}; fn metadata(entry: &TarEntry) -> (Int, Int) { (entry.mode(), entry.mtime()) } fn hard(path: Text, target: Text) -> TarEntry { entry_for_hardlink(path, target, 0o640, 23) } fn soft(path: Text, target: Text) -> TarEntry { entry_for_symlink_with_mode(path, target, 0o700, 29) }",
        95,
    );
    let errors = check("consumer.main", &probe.items);
    assert!(errors.is_empty(), "new public API: {errors:?}");
}

#[test]
fn parent_and_foreign_consumers_cannot_read_tar_metadata_fields() {
    for owner in [ARCHIVE, "consumer.main"] {
        for field in ["mode", "mtime"] {
            let probe = parse(
                &format!("fn probe(entry: &TarEntry) -> Int {{ entry.{field} }}"),
                96,
            );
            let errors = check(owner, &probe.items);
            assert!(
                !errors.is_empty()
                    && errors.iter().all(|error| error.contains("VisibilityError")
                        && error.contains("Private")
                        && error.contains(TAR)
                        && error.contains(field)),
                "{owner} must not read private {field}: {errors:?}"
            );
        }
    }
}

#[test]
fn prepared_generic_runtime_fixture_parses_all_three_controls() {
    let source = include_str!("../../../core-tests/archive/tar/adapter_api_test.vr");
    let ast = parse(source, 97);
    for expected in [
        "generic_roundtrip_preserves_binary_metadata_and_link_targets",
        "original_symlink_constructor_keeps_its_default_mode",
        "unsupported_kinds_refuse_conversion_and_partial_generic_output",
        "main",
    ] {
        assert!(ast.items.iter().any(|item|
            matches!(&item.kind, ItemKind::Function(function) if function.name.name == expected && function.body.is_some())),
            "prepared runtime control {expected}");
    }
}
