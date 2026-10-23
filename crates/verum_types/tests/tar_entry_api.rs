//! T1738: the real TarEntry source owns its private storage and borrowed API.
//!
//! Target records, methods, mounts and function interfaces come from actual
//! source ASTs. Only the selected owner methods and foreign probes are checked;
//! dependency bodies and the archive reader/writer are not executed here.
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

const OWNER: &str = "core.archive.tar";
const SOURCE: &str = include_str!("../../../core/archive/tar.vr");
const API: &[&str] = &["path", "entry_type", "content"];
const FIELDS: &[&str] = &[
    "path",
    "mode",
    "uid",
    "gid",
    "size",
    "mtime",
    "entry_type",
    "uname",
    "gname",
    "content",
];
const IMPORTS: &str = "mount core.archive.tar.{TarEntry, EntryType}; mount core.collections.list.List; mount core.base.maybe.Maybe;";

fn parse(source: &str, file: u32) -> Module {
    FastParser::new()
        .parse_module_str(source, FileId::new(file))
        .expect("actual source or control grammar")
}

fn inherent_owner(item: &Item, name: &str) -> bool {
    matches!(&item.kind, ItemKind::Impl(implementation)
        if matches!(&implementation.kind, ImplKind::Inherent(target)
            if matches!(&target.kind, TypeKind::Path(path)
                if path.as_ident().is_some_and(|ident| ident.name.as_str() == name))))
}

fn dependencies() -> List<(&'static str, &'static str, Module)> {
    // Umbrella modules retain their real re-export paths. Declaration modules
    // retain every original mount, not a Display-string approximation of it.
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
        (OWNER, SOURCE, &["EntryType", "TarEntry", "TarError"][..]),
    ];
    inputs
        .into_iter()
        .enumerate()
        .map(|(index, (owner, source, types))| {
            let mut ast = parse(source, index as u32);
            if !types.is_empty() {
                ast.items.retain(|item| match &item.kind {
                    ItemKind::Mount(_) => true,
                    ItemKind::Type(declaration) => types.contains(&declaration.name.name.as_str()),
                    ItemKind::Impl(_) => owner == OWNER && inherent_owner(item, "TarEntry"),
                    ItemKind::Function(function) => {
                        owner == OWNER
                            && matches!(
                                function.name.name.as_str(),
                                "read_archive"
                                    | "write_archive"
                                    | "entry_for_file"
                                    | "entry_for_directory"
                                    | "entry_for_symlink"
                            )
                    }
                    _ => false,
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

fn checker() -> (TypeChecker, ModuleRegistry) {
    let mut registry = ModuleRegistry::new();
    for (index, (owner, source, ast)) in dependencies().into_iter().enumerate() {
        let id = ModuleId::new(index as u32);
        let path = ModulePath::from_str(owner);
        let mut info = ModuleInfo::new(
            id,
            path.clone(),
            ast.clone(),
            FileId::new(index as u32),
            source.into(),
        );
        info.exports =
            extract_exports_from_module(&ast, id, &path).expect("actual declaration exports");
        registry.register(info);
    }
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog("consumer");
    checker.set_current_module_path("consumer.main");
    checker.set_module_registry_direct(registry.clone());
    let imports = parse(IMPORTS, 80);
    for item in &imports.items {
        if let ItemKind::Mount(import) = &item.kind {
            checker
                .process_import(import, "consumer.main", &registry)
                .expect("real public type imports");
        }
    }
    let errors = diagnostics(&mut checker);
    assert!(
        errors.is_empty(),
        "dependency setup must succeed before any body oracle: {errors:?}"
    );
    (checker, registry)
}

fn check_probe(source: &str) -> List<Text> {
    let (mut checker, registry) = checker();
    let ast = parse(source, 81);
    for item in &ast.items {
        match &item.kind {
            ItemKind::Mount(import) => checker
                .process_import(import, "consumer.main", &registry)
                .expect("probe imports"),
            ItemKind::Function(function) => checker
                .register_function_signature(function)
                .expect("probe signature"),
            _ => {}
        }
    }
    let mut errors = List::new();
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(diagnostics(&mut checker));
    errors
}

fn assert_private(errors: &List<Text>, field: &str) {
    assert!(
        !errors.is_empty()
            && errors.iter().all(|error| error.contains("VisibilityError")
                && error.contains("Private")
                && error.contains(OWNER)
                && error.contains(field)),
        "exact private declaring field {field}: {errors:?}"
    );
}

#[test]
fn original_mounts_and_private_field_policies_survive_selection() {
    let dependencies = dependencies();
    for (index, (_, source, selected)) in dependencies.iter().enumerate() {
        let original = parse(source, index as u32);
        let original_mounts: List<&Item> = original
            .items
            .iter()
            .filter(|item| matches!(item.kind, ItemKind::Mount(_)))
            .collect();
        let selected_mounts: List<&Item> = selected
            .items
            .iter()
            .filter(|item| matches!(item.kind, ItemKind::Mount(_)))
            .collect();
        assert_eq!(
            selected_mounts, original_mounts,
            "complete actual mount ASTs must survive"
        );
    }
    let owner = &dependencies
        .iter()
        .find(|(owner, _, _)| *owner == OWNER)
        .unwrap()
        .2;
    let declaration = owner
        .items
        .iter()
        .find_map(|item| match &item.kind {
            ItemKind::Type(declaration) if declaration.name.name == "TarEntry" => Some(declaration),
            _ => None,
        })
        .expect("actual TarEntry declaration");
    let TypeDeclBody::Record(fields) = &declaration.body else {
        panic!("TarEntry record");
    };
    assert_eq!(fields.len(), FIELDS.len());
    for (field, expected) in fields.iter().zip(FIELDS) {
        assert_eq!(field.name.name.as_str(), *expected);
        assert_eq!(field.visibility, Visibility::Private, "{expected}");
    }
}

#[test]
fn actual_owner_methods_are_public_immutable_borrows_and_check() {
    let ast = parse(SOURCE, 90);
    let implementations: List<&Item> = ast
        .items
        .iter()
        .filter(|item| inherent_owner(item, "TarEntry"))
        .collect();
    let mut found = List::new();
    for item in &implementations {
        let ItemKind::Impl(implementation) = &item.kind else {
            unreachable!()
        };
        for member in &implementation.items {
            let ImplItemKind::Function(function) = &member.kind else {
                continue;
            };
            if !API.contains(&function.name.name.as_str()) {
                continue;
            }
            assert_eq!(member.visibility, Visibility::Public);
            assert!(
                function.body.is_some(),
                "a declaration stub is not an accessor"
            );
            assert!(
                matches!(
                    function.return_type.as_ref().map(|ty| &ty.kind),
                    Some(TypeKind::Reference { mutable: false, .. })
                ),
                "immutable borrowed result: {}",
                function.name.name
            );
            found.push(function.name.name.clone());
        }
    }
    assert_eq!(
        found.len(),
        API.len(),
        "all three API methods must exist: {found:?}"
    );
    for name in API {
        assert!(found.iter().any(|found| found == name));
    }
    let (mut checker, _) = checker();
    checker.set_current_module_path(OWNER);
    for item in implementations {
        checker
            .check_item(item)
            .expect("actual owner accessor body");
    }
    let errors = diagnostics(&mut checker);
    assert!(
        errors.is_empty(),
        "owner method body diagnostics: {errors:?}"
    );
}

#[test]
fn foreign_callers_can_borrow_the_exact_path_kind_and_content_types() {
    let errors = check_probe(
        "fn path(entry: &TarEntry) -> &Text { entry.path() } fn kind(entry: &TarEntry) -> &EntryType { entry.entry_type() } fn bytes(entry: &TarEntry) -> &List<Byte> { entry.content() }",
    );
    assert!(errors.is_empty(), "public borrowed API: {errors:?}");
}

#[test]
fn returned_constructor_value_exposes_the_declared_borrowed_api() {
    // The constructor body is not executed here; its unchanged source signature
    // must retain TarEntry's declaration-owned method in this typed consumer.
    let errors = check_probe(
        "mount core.archive.tar.entry_for_directory; fn probe(path: Text) -> Text { let entry = entry_for_directory(path, 0o755, 0); entry.path().clone() }",
    );
    assert!(errors.is_empty(), "real constructor return: {errors:?}");
}

#[test]
fn foreign_direct_reads_of_all_ten_fields_remain_private() {
    for field in FIELDS {
        let errors = check_probe(&format!("fn probe(entry: &TarEntry) {{ entry.{field}; }}"));
        assert_private(&errors, field);
    }
}

#[test]
fn foreign_direct_write_remains_private() {
    assert_private(
        &check_probe("fn probe(entry: &mut TarEntry) { entry.path = \"changed\"; }"),
        "path",
    );
}

#[test]
fn foreign_literal_cannot_forge_a_tar_entry() {
    let errors = check_probe(
        "fn probe(path: Text, ids: Maybe<Int>, kind: EntryType, content: List<Byte>) -> TarEntry { TarEntry { path: path, mode: 0, uid: ids, gid: ids, size: 0, mtime: 0, entry_type: kind, uname: \"\", gname: \"\", content: content } }",
    );
    assert_private(&errors, "path");
}

#[test]
fn borrowed_api_does_not_grant_mutable_references() {
    for (method, result) in [
        ("path", "Text"),
        ("entry_type", "EntryType"),
        ("content", "List<Byte>"),
    ] {
        let errors = check_probe(&format!(
            "fn probe(entry: &TarEntry) -> &mut {result} {{ entry.{method}() }}"
        ));
        assert!(
            !errors.is_empty()
                && errors
                    .iter()
                    .all(|error| error.contains("Mismatch") || error.contains("Mutab")),
            "immutable {method} borrow must not become mutable: {errors:?}"
        );
    }
}
