//! Deferred refinement-name checks retain their declaring lexical authority.
use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::VerumParser;
use verum_modules::{
    ModuleInfo, ModulePath, ModuleRegistry, SharedModuleRegistry, extract_exports_from_module,
    resolve_glob_reexports, resolve_specific_reexport_kinds,
};
use verum_types::TypeChecker;

fn errors(dependencies: &[(&str, &str)], source: &str) -> List<Text> {
    let mut registry = ModuleRegistry::new();
    for (owner, source) in dependencies.iter().copied().chain([("consumer", source)]) {
        let id = registry.allocate_id();
        let file = FileId::new(id.as_u32());
        let ast = VerumParser::new()
            .parse_module_str(source, file)
            .expect("fixture grammar");
        let path = ModulePath::from_str(owner);
        let exports = extract_exports_from_module(&ast, id, &path).unwrap();
        let mut info = ModuleInfo::new(id, path, ast, file, source.into());
        info.exports = exports;
        registry.register(info);
    }
    resolve_glob_reexports(&mut registry).unwrap();
    resolve_specific_reexport_kinds(&mut registry).unwrap();
    let module = registry.get_by_path("consumer").unwrap().clone();
    let mut checker =
        TypeChecker::with_shared_registry(SharedModuleRegistry::from_registry(registry));
    checker.register_builtins();
    checker.set_current_module(module.id);
    checker.set_current_module_path("consumer");
    let mut errors = checker.prepare_module_declarations(&module.ast);
    for item in &module.ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error);
        }
    }
    errors.extend(checker.take_deferred_errors());
    errors
        .iter()
        .chain(checker.diagnostic_sources())
        .map(|error| Text::from(format!("{error:?}")))
        .collect()
}

const CONSUMER: &str = "mount dependency.Bounded; fn read(value: Bounded) {}";

#[test]
fn imported_refinements_resolve_private_forward_constants_in_the_declaring_file() {
    let found = errors(
        &[(
            "dependency",
            "public type Bounded is Float{it >= -LIMIT && it < LIMIT}; const LIMIT: Float = 3.141592653589793;",
        )],
        CONSUMER,
    );
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn a_consumers_constant_cannot_supply_a_missing_dependency_name() {
    for consumer in [
        "const LIMIT: Int = 10; mount dependency.Bounded; fn read(value: Bounded) {}",
        "mount dependency.Bounded; const LIMIT: Int = 10; fn read(value: Bounded) {}",
    ] {
        let found = errors(
            &[("dependency", "public type Bounded is Int{it < LIMIT};")],
            consumer,
        );
        assert!(
            found
                .iter()
                .any(|e| e.contains("unbound variable in refinement predicate: `LIMIT`")),
            "{found:#?}"
        );
    }
}

#[test]
fn a_known_forward_name_does_not_hide_another_unknown_name() {
    let found = errors(
        &[],
        "type Bounded is Int{it > MISSING && it < LIMIT}; const LIMIT: Int = 10;",
    );
    assert!(
        found
            .iter()
            .any(|e| e.contains("unbound variable in refinement predicate: `MISSING`")),
        "{found:#?}"
    );
    assert!(
        !found
            .iter()
            .any(|e| e.contains("unbound variable in refinement predicate: `LIMIT`")),
        "{found:#?}"
    );
}

#[test]
fn private_named_mounts_and_public_reexports_retain_their_names() {
    let found = errors(
        &[
            ("limits", "public const MAX: Int = 10;"),
            ("facade", "public mount limits.{MAX as UPPER};"),
            (
                "dependency",
                "mount facade.{UPPER as LIMIT}; public type Bounded is Int{it < LIMIT};",
            ),
        ],
        CONSUMER,
    );
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn missing_or_private_mount_targets_cannot_bind_a_predicate() {
    for declaration in ["const LIMIT: Int = 10;", "public const OTHER: Int = 10;"] {
        let found = errors(
            &[
                ("limits", declaration),
                (
                    "dependency",
                    "mount limits.LIMIT; public type Bounded is Int{it < LIMIT};",
                ),
            ],
            CONSUMER,
        );
        assert!(
            found
                .iter()
                .any(|e| e.contains("unbound variable in refinement predicate: `LIMIT`")),
            "{found:#?}"
        );
    }
}

#[test]
fn inline_refinements_keep_their_own_constants() {
    let found = errors(
        &[],
        "module nested { public type Bounded is Int{it < LIMIT}; const LIMIT: Int = 10; } mount consumer.nested.Bounded; fn read(value: Bounded) {}",
    );
    assert!(found.is_empty(), "{found:#?}");
}
