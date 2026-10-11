//! T1737: actual SemVer declarations and selected sibling bodies keep owners.
//!
//! This is a source-body checker gate, not a complete stdlib dependency check.
//! Every selected declaration, interface and target body is parsed unchanged
//! from its complete current source file. Dependency bodies are not checked.
//! No field policy, method body or metadata is fabricated to make a call pass.
use verum_ast::{
    FileId, Item, ItemKind, Module, Visibility,
    decl::{ImplItemKind, ImplKind, TypeDeclBody},
    ty::{Path as AstPath, PathSegment, TypeKind},
};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::TypeChecker;

const OWNER: &str = "core.base.semver";
const CALLER: &str = "core.base.semver_constraint";
const API: &[&str] = &[
    "major",
    "minor",
    "patch",
    "prerelease_identifiers",
    "compare_prerelease",
];
const VERSION: &str = include_str!("../../../core/base/semver.vr");
const REQUIREMENT: &str = include_str!("../../../core/base/semver_constraint.vr");

fn parse(source: &str, file: u32) -> Module {
    FastParser::new()
        .parse_module_str(source, FileId::new(file))
        .expect("actual source grammar")
}

fn named_path_is(path: &AstPath, expected: &[&str]) -> bool {
    path.segments.len() == expected.len()
        && path.segments.iter().zip(expected).all(|(segment, expected)| {
            matches!(segment, PathSegment::Name(name) if name.name.as_str() == *expected)
        })
}

fn selected(
    source: &str,
    file: u32,
    types: &[&str],
    methods: &[&str],
    functions: &[&str],
) -> Module {
    let mut ast = parse(source, file);
    ast.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Type(declaration) => types.contains(&declaration.name.name.as_str()),
        ItemKind::Function(function) => functions.contains(&function.name.name.as_str()),
        ItemKind::Mount(import) => matches!(&import.tree.kind,
            verum_ast::MountTreeKind::Nested { prefix, .. }
                if named_path_is(prefix, &["core", "collections", "list"])
                    || named_path_is(prefix, &["collections", "list"])
                    || named_path_is(prefix, &["base", "semver"])
        ),
        ItemKind::Impl(implementation) => {
            // A same-named method on ListIter/Drain is not List's interface.
            let target = match &implementation.kind {
                ImplKind::Inherent(target) => target,
                ImplKind::Protocol { for_type, .. } => for_type,
            };
            let target = match &target.kind {
                TypeKind::Generic { base, .. } => base.as_ref(),
                _ => target,
            };
            let selected_target = types.first().copied().unwrap_or("Text");
            let selected_receiver = match &target.kind {
                TypeKind::Path(path) => path.as_ident().is_some_and(|name| name.name == selected_target),
                TypeKind::Text => selected_target == "Text",
                _ => false,
            };
            if !selected_receiver {
                return false;
            }
            implementation.items.retain(|item| {
                matches!(&item.kind, ImplItemKind::Function(function) if methods.contains(&function.name.name.as_str()))
            });
            !implementation.items.is_empty()
        }
        _ => false,
    });
    ast
}

fn modules() -> List<(&'static str, &'static str, Module)> {
    let inputs = [
        (
            "core.base.ordering",
            include_str!("../../../core/base/ordering.vr"),
            &["Ordering"][..],
            &[][..],
            &[][..],
        ),
        (
            "core.base.protocols",
            include_str!("../../../core/base/protocols.vr"),
            &["PartialEq", "Eq", "Ord", "Clone"][..],
            &[][..],
            &[][..],
        ),
        (
            "core.base.maybe",
            include_str!("../../../core/base/maybe.vr"),
            &["Maybe"][..],
            &[][..],
            &[][..],
        ),
        (
            "core.base.memory",
            include_str!("../../../core/base/memory.vr"),
            &["Heap"][..],
            &[][..],
            &[][..],
        ),
        (
            "core.collections.list",
            include_str!("../../../core/collections/list.vr"),
            &["List"][..],
            &["len", "is_empty", "clone"][..],
            &[][..],
        ),
        (
            "core.text.text",
            include_str!("../../../core/text/text.vr"),
            &[][..],
            &["as_bytes", "clone"][..],
            &[][..],
        ),
        (
            OWNER,
            VERSION,
            &["SemVer", "SemVerError"][..],
            &[
                "major",
                "minor",
                "patch",
                "prerelease_identifiers",
                "compare_prerelease",
                "cmp",
                "eq",
            ][..],
            &[
                "compare",
                "compare_prerelease_identifiers",
                "cmp_prerelease_id",
                "cmp_int",
                "cmp_u64",
                "is_all_numeric",
            ][..],
        ),
        (
            CALLER,
            REQUIREMENT,
            &[
                "SemVerOperator",
                "SemVerComparator",
                "SemVerConstraint",
                "MatchOptions",
            ][..],
            &[][..],
            &[
                "comparator_from_version",
                "prerelease_order",
                "compatible_operand",
                "compatible_comparator",
            ][..],
        ),
    ];
    inputs
        .into_iter()
        .enumerate()
        .map(|(index, (owner, source, types, methods, functions))| {
            (
                owner,
                source,
                selected(source, index as u32, types, methods, functions),
            )
        })
        .collect()
}

fn diagnostics(checker: &mut TypeChecker, errors: &mut List<Text>) {
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| format!("{e:?}").into()),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|e| format!("{e:?}").into()),
    );
}

fn setup() -> (
    TypeChecker,
    ModuleRegistry,
    List<(&'static str, &'static str, Module)>,
) {
    let modules = modules();
    let mut registry = ModuleRegistry::new();
    for (index, (owner, source, ast)) in modules.iter().enumerate() {
        let id = ModuleId::new(index as u32);
        let path = ModulePath::from_str(owner);
        let mut info = ModuleInfo::new(
            id,
            path.clone(),
            ast.clone(),
            FileId::new(index as u32),
            (*source).into(),
        );
        info.exports = extract_exports_from_module(ast, id, &path).expect("actual exports");
        registry.register(info);
    }
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_module_registry_direct(registry.clone());
    let mut errors = List::new();
    for (owner, _, ast) in &modules {
        checker.set_current_module_path(*owner);
        for item in &ast.items {
            if let ItemKind::Mount(import) = &item.kind {
                if let Err(error) = checker.process_import(import, owner, &registry) {
                    errors.push(format!("{owner} mount: {error:?}").into());
                }
            }
        }
        for item in &ast.items {
            if let ItemKind::Type(declaration) = &item.kind {
                checker.register_type_name_only(declaration);
            }
        }
        for item in &ast.items {
            let result = match &item.kind {
                ItemKind::Type(declaration) => checker.register_type_declaration(declaration),
                ItemKind::Impl(declaration) => checker.register_impl_block(declaration),
                ItemKind::Function(function) => checker.register_function_signature(function),
                _ => continue,
            };
            if let Err(error) = result {
                errors.push(format!("{owner}: {error:?}").into());
            }
        }
    }
    diagnostics(&mut checker, &mut errors);
    assert!(
        errors.is_empty(),
        "actual dependency interfaces must register before the body oracle: {errors:?}"
    );
    (checker, registry, modules)
}

fn check_items(owner: &str, items: &[Item]) -> List<Text> {
    let (mut checker, _, _) = setup();
    checker.set_current_module_path(owner);
    let mut errors = List::new();
    for item in items {
        if let ItemKind::Function(function) = &item.kind {
            checker
                .register_function_signature(function)
                .expect("probe signature");
        }
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    diagnostics(&mut checker, &mut errors);
    if !errors.is_empty() {
        for (owner, method) in [
            ("core.base.semver.SemVer", "prerelease_identifiers"),
            ("core.collections.list.List", "is_empty"),
            ("List", "is_empty"),
        ] {
            eprintln!(
                "source interface {owner}.{method}: {:?}",
                checker.lookup_instance_method_for_testing(owner, method)
            );
        }
    }
    errors
}

fn actual_function(owner: &str, name: &str) -> Item {
    modules()
        .into_iter()
        .find(|(module, _, _)| *module == owner)
        .unwrap()
        .2
        .items
        .into_iter()
        .find(
            |item| matches!(&item.kind, ItemKind::Function(function) if function.name.name == name),
        )
        .unwrap_or_else(|| panic!("actual function {owner}.{name} must exist"))
}

#[test]
fn selected_dependency_mounts_keep_source_items_and_public_exports() {
    let modules = modules();
    for (owner, prefixes) in [
        (OWNER, &[&["core", "collections", "list"][..]][..]),
        (
            CALLER,
            &[&["collections", "list"][..], &["base", "semver"][..]][..],
        ),
    ] {
        let (index, (_, source, selected)) = modules
            .iter()
            .enumerate()
            .find(|(_, (name, _, _))| *name == owner)
            .expect("selected actual module");
        let original = parse(source, index as u32);
        let retained: List<&Item> = selected
            .items
            .iter()
            .filter(|item| matches!(item.kind, ItemKind::Mount(_)))
            .collect();
        assert_eq!(retained.len(), prefixes.len(), "{owner} actual mounts");
        for prefix in prefixes {
            let original_item = original
                .items
                .iter()
                .find(|item| {
                    matches!(&item.kind,
                    ItemKind::Mount(import) if matches!(&import.tree.kind,
                        verum_ast::MountTreeKind::Nested { prefix: actual, .. }
                            if named_path_is(actual, prefix)))
                })
                .expect("required mount exists in unchanged source");
            assert!(
                retained.iter().any(|item| *item == original_item),
                "{owner} must retain the complete original mount {prefix:?}"
            );
        }
    }
    for (owner, names) in [
        ("core.collections.list", &["List"][..]),
        (OWNER, &["SemVer", "SemVerError"][..]),
    ] {
        let (index, (_, _, ast)) = modules
            .iter()
            .enumerate()
            .find(|(_, (name, _, _))| *name == owner)
            .expect("actual dependency module");
        let exports = extract_exports_from_module(
            ast,
            ModuleId::new(index as u32),
            &ModulePath::from_str(owner),
        )
        .expect("actual dependency exports");
        for name in names {
            let export = exports
                .get(&Text::from(*name))
                .expect("mounted type export");
            assert_eq!(export.kind, verum_modules::ExportKind::Type);
            assert_eq!(export.visibility, Visibility::Public);
        }
    }
}

#[test]
fn semver_keeps_all_five_private_fields() {
    let ast = parse(VERSION, 90);
    let declaration = ast
        .items
        .iter()
        .find_map(|item| match &item.kind {
            ItemKind::Type(declaration) if declaration.name.name == "SemVer" => Some(declaration),
            _ => None,
        })
        .unwrap();
    let TypeDeclBody::Record(fields) = &declaration.body else {
        panic!("SemVer record");
    };
    assert_eq!(fields.len(), 5);
    for field in fields {
        assert_eq!(field.visibility, Visibility::Private, "{}", field.name.name);
    }
}

#[test]
fn actual_semver_read_only_methods_check_inside_the_declaring_owner() {
    let ast = modules()
        .into_iter()
        .find(|(owner, _, _)| *owner == OWNER)
        .unwrap()
        .2;
    let mut names: List<Text> = List::new();
    let implementations: List<Item> = ast.items.into_iter().filter(|item| {
        if let ItemKind::Impl(implementation) = &item.kind
            && matches!(&implementation.kind, ImplKind::Inherent(target) if matches!(&target.kind, TypeKind::Path(path) if path.as_ident().is_some_and(|name| name.name == "SemVer")))
        {
            for item in &implementation.items {
                if let ImplItemKind::Function(function) = &item.kind {
                    assert_eq!(item.visibility, Visibility::Public);
                    assert!(function.body.is_some());
                    names.push(function.name.name.clone());
                }
            }
            true
        } else { false }
    }).collect();
    assert_eq!(
        names.len(),
        API.len(),
        "all declared public read-only bodies must exist: {names:?}"
    );
    for name in API {
        assert!(names.iter().any(|actual| actual == name));
    }
    let errors = check_items(OWNER, &implementations);
    assert!(errors.is_empty(), "owner API bodies: {errors:?}");
}

macro_rules! actual_caller {
    ($test:ident, $name:literal) => {
        #[test]
        fn $test() {
            let errors = check_items(CALLER, &[actual_function(CALLER, $name)]);
            assert!(
                errors.is_empty(),
                "actual sibling body {} must use the legal API: {errors:?}",
                $name
            );
        }
    };
}
actual_caller!(
    actual_constraint_constructor_respects_private_components,
    "comparator_from_version"
);
actual_caller!(
    actual_prerelease_order_respects_private_components,
    "prerelease_order"
);
actual_caller!(
    actual_operand_admission_respects_private_components,
    "compatible_operand"
);
actual_caller!(
    actual_comparator_admission_respects_private_components,
    "compatible_comparator"
);

fn foreign_errors(source: &str) -> List<Text> {
    check_items("consumer", &parse(source, 91).items)
}

fn assert_private(errors: List<Text>, field: &str) {
    assert!(
        !errors.is_empty()
            && errors.iter().all(|error| {
                error.contains("VisibilityError")
                    && error.contains("Private")
                    && error.contains(OWNER)
                    && error.contains(field)
            }),
        "exact private owner/field refusal: {errors:?}"
    );
}

#[test]
fn foreign_direct_component_reads_remain_private() {
    for field in ["major", "minor", "patch"] {
        assert_private(
            foreign_errors(&format!(
                "fn probe(value: &core.base.semver.SemVer) -> UInt64 {{ value.{field} }}"
            )),
            field,
        );
    }
    for field in ["pre_release", "build_meta"] {
        assert_private(
            foreign_errors(&format!(
                "fn probe(value: &core.base.semver.SemVer) -> &List<Text> {{ &value.{field} }}"
            )),
            field,
        );
    }
}

#[test]
fn foreign_literal_construction_remains_private() {
    assert_private(
        foreign_errors(
            "fn probe(ids: List<Text>, build: List<Text>) -> core.base.semver.SemVer { core.base.semver.SemVer { major: 1_u64, minor: 2_u64, patch: 3_u64, pre_release: ids, build_meta: build } }",
        ),
        "major",
    );
}
