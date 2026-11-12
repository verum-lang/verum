//! Re-export copies retain declaration identity and unverified contract payloads.
use verum_ast::FileId;
use verum_common::Text;
use verum_fast_parser::VerumParser;
use verum_modules::{
    ExportTable, ModuleId, ModuleInfo, ModulePath, ModuleRegistry, Visibility,
    extract_exports_from_module,
    refinement_info::{RefinementContract, RefinementInfo},
    resolve_glob_reexports, resolve_specific_reexport_kinds,
};

fn module(registry: &ModuleRegistry, owner: &str, source: &str) -> ModuleInfo {
    let id = registry.allocate_id();
    let file = FileId::new(id.as_u32());
    let ast = VerumParser::new().parse_module_str(source, file).unwrap();
    let path = ModulePath::from_str(owner);
    let exports = extract_exports_from_module(&ast, id, &path).unwrap();
    let mut info = ModuleInfo::new(id, path, ast, file, source.into());
    info.exports = exports;
    info
}

fn register(registry: &mut ModuleRegistry, owner: &str, source: &str) -> ModuleId {
    let info = module(registry, owner, source);
    registry.register(info)
}

fn add_glob_routes(registry: &mut ModuleRegistry) {
    register(registry, "chain", "public mount provider.*;");
    register(
        registry,
        "facade",
        "public module surface { public mount chain.*; }",
    );
    register(registry, "facade.surface", "public mount chain.*;");
}

#[test]
fn renamed_types_and_restricted_visibility_survive_globs_and_inline_lifting() {
    let mut registry = ModuleRegistry::new();
    let original = register(
        &mut registry,
        "original",
        "public type Value is { public value: Int }; type Hidden is (); public(cog) type Local is ();",
    );
    register(
        &mut registry,
        "provider",
        "public mount original.*; public mount original.{Value as Alias};",
    );
    add_glob_routes(&mut registry);
    resolve_specific_reexport_kinds(&mut registry).unwrap();
    let expected_alias = registry
        .get_by_path("provider")
        .unwrap()
        .exports
        .get(&Text::from("Alias"))
        .unwrap()
        .clone();
    assert_eq!(expected_alias.original_name.as_deref(), Some("Value"));
    assert_eq!(expected_alias.source_module, original);
    resolve_glob_reexports(&mut registry).unwrap();
    for owner in ["provider", "chain", "facade.surface", "facade"] {
        let info = registry.get_by_path(owner).unwrap();
        assert_eq!(
            info.exports.get(&Text::from("Alias")),
            Some(&expected_alias),
            "{owner}"
        );
        assert!(!info.exports.contains("Hidden"), "{owner}");
        let restricted = info.exports.get(&Text::from("Local")).unwrap();
        assert_eq!(restricted.source_module, original);
        assert_eq!(restricted.visibility, Visibility::PublicCrate, "{owner}");
        assert!(
            !info
                .exports
                .public_exports()
                .any(|item| item.name == "Local"),
            "{owner}"
        );
    }
}

#[test]
fn propagated_metadata_preserves_predicates_without_claiming_they_are_proved() {
    let mut registry = ModuleRegistry::new();
    let mut provider = module(
        &registry,
        "provider",
        "public type Positive is Int; public fn advance(input: Int) -> Int { input + 1 }",
    );
    let file = provider.file_id;
    let predicate = VerumParser::new()
        .parse_expr_str("value > 0", file)
        .unwrap();
    let predicate_span = predicate.span;
    let refined = provider
        .exports
        .get(&Text::from("Positive"))
        .unwrap()
        .clone()
        .with_refinement(
            RefinementInfo::new(
                verum_ast::ty::Type::int(predicate_span),
                predicate,
                Some("value".into()),
                predicate_span,
            ),
            Visibility::Private,
        );
    let mut contract = RefinementContract::new();
    for (source, precondition) in [("input > 0", true), ("result > input", false)] {
        let predicate = VerumParser::new().parse_expr_str(source, file).unwrap();
        let span = predicate.span;
        if precondition {
            contract.add_requires(predicate, span);
        } else {
            contract.add_ensures(predicate, span);
        }
    }
    let contracted = provider
        .exports
        .get(&Text::from("advance"))
        .unwrap()
        .clone()
        .with_contract(contract);
    // This exercises the payload supplied by a metadata producer. No verified
    // status is assigned: copying declarations must preserve pending checks.
    let mut exports = ExportTable::new();
    exports.set_module_id(provider.id);
    exports.set_module_path(provider.path.clone());
    exports.add_export(refined.clone()).unwrap();
    exports.add_export(contracted.clone()).unwrap();
    provider.exports = exports;
    registry.register(provider);
    add_glob_routes(&mut registry);
    resolve_glob_reexports(&mut registry).unwrap();
    for owner in ["chain", "facade.surface", "facade"] {
        let info = registry.get_by_path(owner).unwrap();
        assert_eq!(info.exports.get(&refined.name), Some(&refined), "{owner}");
        assert_eq!(
            info.exports.get(&contracted.name),
            Some(&contracted),
            "{owner}"
        );
        assert!(
            info.exports
                .get(&contracted.name)
                .unwrap()
                .needs_runtime_contract_check()
        );
        assert_eq!(
            info.exports
                .get(&refined.name)
                .unwrap()
                .predicate_visibility,
            Visibility::Private
        );
    }
}
