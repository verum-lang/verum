//! T1694: parsed declaring files → actual bootstrap unit → archive wire → metadata.
//! Sibling mounts must not become an alias declaration's namespace authority.
use super::*;
use crate::Session;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::core_metadata::TypeDescriptorKind;
use verum_vbc::{
    VbcModule,
    archive::{ArchiveBuilder, VbcArchive},
    types::{TypeDescriptor, TypeId, TypeParamId, TypeRef},
};

const ALIAS: (&str, &str) = (
    "fixture.protocols",
    "public type IoResult<T> is Result<T, Int>;",
);
const MISSING_ROOT: (&str, &str) = ("fixture.buffer", "mount core.Result;");

fn bootstrap(files: &[(&str, &str)], root_reexport: bool) -> (VbcModule, VbcArchive) {
    let mut units: List<(&str, List<(&str, &str)>)> = List::from_iter([
        (
            "core.base.result",
            List::from_iter([(
                "core.base.result",
                "public type Result<T, E> is Ok(T) | Err(E);",
            )]),
        ),
        (
            "core.base",
            List::from_iter([("core.base", "public mount .result.Result;")]),
        ),
        (
            "foreign.result",
            List::from_iter([(
                "foreign.result",
                "public type Result<T, E> is ForeignOk(T) | ForeignErr(E);",
            )]),
        ),
    ]);
    if root_reexport {
        units.push((
            "core",
            List::from_iter([("core", "public mount .base.result.Result;")]),
        ));
    }
    units.push(("fixture", files.iter().copied().collect()));
    let parsed: List<_> = units
        .iter()
        .map(|(bundle, files)| {
            let files: List<_> = files
                .iter()
                .map(|(owner, source)| {
                    let ast = Parser::new(&format!("module {owner}; {source}"))
                        .parse_module()
                        .expect("fixture follows the source grammar");
                    (*owner, ast)
                })
                .collect();
            (*bundle, files)
        })
        .collect();
    let mut session = Session::new(Default::default());
    let config = CoreConfig::new(".");
    // Match compile_core: all parsed file exports exist before any unit is
    // produced. A genuine root/umbrella re-export must remain distinguishable
    // from a nonexistent path; no handcrafted descriptor supplies its target.
    {
        let registry = session.module_registry();
        let mut registry = registry.write();
        for (_, files) in &parsed {
            for (owner, ast) in files {
                let id = registry.allocate_id();
                let path = ModulePath::from_str(owner);
                let mut info =
                    ModuleInfo::new(id, path.clone(), ast.clone(), ast.file_id, Text::new());
                info.exports = extract_exports_from_module(ast, id, &path).unwrap();
                registry.register(info);
            }
        }
        resolve_specific_reexport_kinds(&mut registry).unwrap();
        resolve_glob_reexports(&mut registry).unwrap();
    }
    let mut pipeline = CompilationPipeline::new_core(&mut session, config.clone());
    let mut builder = ArchiveBuilder::new();
    let mut produced = None;
    for (bundle, files) in &parsed {
        let unit = StdlibModule {
            name: (*bundle).into(),
            source_files: List::new().into(),
            dependencies: List::new().into(),
        };
        let asts: List<_> = files.iter().map(|(_, ast)| ast).collect();
        let (module, _) = pipeline
            .compile_core_module_from_ast(
                &unit,
                asts.as_slice(),
                &config,
                &verum_ast::cfg::TargetConfig::host(),
                &Default::default(),
            )
            .expect("actual bootstrap producer accepts the source unit");
        builder.add_module(bundle, &module, &[]).unwrap();
        pipeline
            .compiled_stdlib_modules
            .insert((*bundle).into(), module.clone());
        produced = Some(module);
    }
    let mut bytes = List::new();
    verum_vbc::archive::write_archive(&builder.finish(), &mut bytes).unwrap();
    let archive = verum_vbc::archive::read_archive(std::io::Cursor::new(bytes.as_slice())).unwrap();
    (produced.unwrap(), archive)
}

fn declaration<'a>(module: &'a VbcModule, owner: &str, leaf: &str) -> &'a TypeDescriptor {
    module
        .types
        .iter()
        .find(|ty| {
            ty.origin_module.and_then(|id| module.get_string(id)) == Some(owner)
                && module
                    .get_string(ty.name)
                    .is_some_and(|name| name.rsplit('.').next() == Some(leaf))
        })
        .unwrap_or_else(|| panic!("missing declaration {owner}.{leaf}"))
}

fn target(module: &VbcModule, owner: &str, leaf: &str) -> TypeRef {
    let ty = declaration(module, owner, leaf);
    eprintln!(
        "alias {owner}.{leaf}: target={:?}, carried={:?}",
        ty.alias_target,
        ty.alias_target_name.and_then(|id| module.get_string(id)),
    );
    ty.alias_target.clone().expect("alias target")
}

fn expected(base: TypeId, error: TypeId) -> TypeRef {
    TypeRef::Instantiated {
        base,
        args: List::from_iter([TypeRef::Generic(TypeParamId(0)), TypeRef::Concrete(error)]).into(),
    }
}

fn assert_canonical(files: &[(&str, &str)], root_reexport: bool) {
    let (module, archive) = bootstrap(files, root_reexport);
    let decoded = archive.load_module("fixture").unwrap();
    for module in [&module, &decoded] {
        assert_eq!(
            target(module, ALIAS.0, "IoResult"),
            expected(TypeId::RESULT, TypeId::INT),
            "declaring-file identity must survive source collection and wire",
        );
    }
    let metadata = crate::archive_metadata::archive_to_core_metadata(&archive);
    let TypeDescriptorKind::Alias { target } =
        &metadata.types[&Text::from("fixture.protocols.IoResult")].kind
    else {
        panic!("metadata must retain the alias declaration");
    };
    assert_eq!(target.as_str(), "Result<T, Int>");
}

#[test]
fn unmounted_alias_keeps_canonical_head_with_unrelated_same_leaf_available() {
    assert_canonical(&[ALIAS], false);
}

#[test]
fn missing_root_mount_in_earlier_sibling_cannot_retarget_alias() {
    assert_canonical(&[MISSING_ROOT, ALIAS], false);
}

#[test]
fn missing_root_mount_in_later_sibling_cannot_retarget_alias() {
    assert_canonical(&[ALIAS, MISSING_ROOT], false);
}

#[test]
fn valid_root_reexport_resolves_in_its_own_declaring_file() {
    assert_canonical(
        &[(
            ALIAS.0,
            "mount core.Result; public type IoResult<T> is Result<T, Int>;",
        )],
        true,
    );
}

#[test]
fn exact_and_umbrella_mounts_resolve_in_their_declaring_file() {
    for mount in ["core.base.result.Result", "core.base.Result"] {
        let source = format!("mount {mount}; {}", ALIAS.1);
        assert_canonical(&[(ALIAS.0, &source)], false);
    }
}

#[test]
fn valid_root_reexport_in_sibling_preserves_both_file_orders() {
    assert_canonical(&[MISSING_ROOT, ALIAS], true);
    assert_canonical(&[ALIAS, MISSING_ROOT], true);
}

#[test]
fn foreign_same_leaf_mount_is_local_to_its_declaring_file() {
    let foreign = (
        "fixture.foreign_consumer",
        "mount foreign.result.Result; public type ForeignAlias<T> is Result<T, Bool>;",
    );
    for files in [[foreign, ALIAS], [ALIAS, foreign]] {
        let (module, archive) = bootstrap(&files, false);
        let decoded = archive.load_module("fixture").unwrap();
        for module in [&module, &decoded] {
            let foreign_id = declaration(module, "foreign.result", "Result").id;
            assert_ne!(foreign_id, TypeId::RESULT);
            assert_eq!(
                target(module, foreign.0, "ForeignAlias"),
                expected(foreign_id, TypeId::BOOL),
                "explicit foreign mount retains its exact declared head",
            );
            assert_eq!(
                target(module, ALIAS.0, "IoResult"),
                expected(TypeId::RESULT, TypeId::INT),
                "unmounted sibling does not inherit the foreign head",
            );
        }
    }
}

#[test]
fn exact_mounts_to_same_leaf_declarations_keep_both_owners_in_both_orders() {
    let canonical = (
        ALIAS.0,
        "mount core.base.result.Result; public type IoResult<T> is Result<T, Int>;",
    );
    let foreign = (
        "fixture.foreign_consumer",
        "mount foreign.result.Result; public type ForeignAlias<T> is Result<T, Bool>;",
    );
    for files in [[foreign, canonical], [canonical, foreign]] {
        let (module, archive) = bootstrap(&files, false);
        let decoded = archive.load_module("fixture").unwrap();
        for module in [&module, &decoded] {
            assert_eq!(
                target(module, ALIAS.0, "IoResult"),
                expected(TypeId::RESULT, TypeId::INT)
            );
            let foreign_id = declaration(module, "foreign.result", "Result").id;
            assert_ne!(foreign_id, TypeId::RESULT);
            assert_eq!(
                target(module, foreign.0, "ForeignAlias"),
                expected(foreign_id, TypeId::BOOL)
            );
        }
    }
}

#[test]
fn missing_owner_mount_never_borrows_a_known_generic_head() {
    let (module, archive) = bootstrap(
        &[(
            ALIAS.0,
            "mount missing.owner.Result; public type IoResult<T> is Result<T, Int>;",
        )],
        false,
    );
    let decoded = archive.load_module("fixture").unwrap();
    for module in [&module, &decoded] {
        let TypeRef::Instantiated { base, .. } = target(module, ALIAS.0, "IoResult") else {
            panic!("generic target");
        };
        // This is an identity-refusal control, not a claim that lenient
        // bootstrap validates an unknown source type (separate T0811).
        assert_ne!(base, TypeId::RESULT);
        assert!(!module.types.iter().any(|ty| ty.id == base
            && ty.origin_module.and_then(|id| module.get_string(id)) == Some("foreign.result")));
    }
}

#[test]
fn local_generic_declaration_shadows_a_sibling_mount_in_both_orders() {
    let local = (
        ALIAS.0,
        "public type Result<T, E> is LocalOk(T) | LocalErr(E); public type IoResult<T> is Result<T, Int>;",
    );
    let foreign = ("fixture.buffer", "mount foreign.result.Result;");
    for files in [[foreign, local], [local, foreign]] {
        let (module, archive) = bootstrap(&files, false);
        let decoded = archive.load_module("fixture").unwrap();
        for module in [&module, &decoded] {
            let local_id = declaration(module, ALIAS.0, "Result").id;
            assert_ne!(local_id, TypeId::RESULT);
            assert_eq!(
                target(module, ALIAS.0, "IoResult"),
                expected(local_id, TypeId::INT)
            );
        }
    }
}

#[test]
fn alias_keeps_declared_generic_slot_and_local_error_identity() {
    let (module, archive) = bootstrap(
        &[(
            ALIAS.0,
            "public type Fault is { code: Int }; public type IoResult<T> is Result<T, Fault>;",
        )],
        false,
    );
    let decoded = archive.load_module("fixture").unwrap();
    for module in [&module, &decoded] {
        let fault = declaration(module, ALIAS.0, "Fault").id;
        assert_eq!(
            target(module, ALIAS.0, "IoResult"),
            expected(TypeId::RESULT, fault)
        );
    }
}

#[test]
fn genuine_usize_alias_remains_a_scalar_carrier() {
    let (module, archive) = bootstrap(&[(ALIAS.0, "public type Address<T> is USize;")], false);
    let decoded = archive.load_module("fixture").unwrap();
    for module in [&module, &decoded] {
        assert_eq!(
            target(module, ALIAS.0, "Address"),
            TypeRef::Concrete(TypeId::USIZE)
        );
    }
}
