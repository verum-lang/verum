//! Imported source signatures resolve the declaring file's actual implicit prelude.
use parking_lot::RwLock;
use verum_ast::FileId;
use verum_common::{Shared, Text};
use verum_fast_parser::FastParser;
use verum_modules::{ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
use verum_types::TypeChecker;

fn errors(
    provider_prefix: &str,
    seed_visibility: &str,
    prelude_mount: &str,
    result_type: &str,
) -> Vec<String> {
    let provider = format!("{provider_prefix} public type Packet is {{ public value: Pivot }};");
    let seed =
        format!("module realm.seed; {seed_visibility} type Pivot is {{ public code: Int }};");
    let prelude = format!("module core.prelude; {prelude_mount}");
    let client = format!(
        "module realm.client; mount realm.provider.Packet; fn read(packet: &Packet) -> {result_type} {{ packet.value.code }}"
    );
    let files = [
        ("realm.provider", provider),
        ("realm.seed", seed),
        ("core.prelude", prelude),
        ("realm.client", client),
    ];
    let asts: Vec<_> = files
        .iter()
        .enumerate()
        .map(|(i, (_, source))| {
            FastParser::new()
                .parse_module_str(source, FileId::new(i as u32))
                .unwrap()
        })
        .collect();
    let mut registry = ModuleRegistry::new();
    for ((owner, _), ast) in files.iter().zip(&asts) {
        let id = registry.allocate_id();
        let path = ModulePath::from_str(owner);
        let mut module = ModuleInfo::new(id, path.clone(), ast.clone(), ast.file_id, Text::new());
        module.exports = extract_exports_from_module(ast, id, &path).unwrap();
        registry.register(module);
    }
    verum_modules::resolve_specific_reexport_kinds(&mut registry).unwrap();
    verum_modules::resolve_glob_reexports(&mut registry).unwrap();
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_builtins();
    checker.set_current_module_path("realm.client");
    checker.set_module_registry(Shared::new(RwLock::new(registry)));
    let mut client = asts[3].clone();
    client.items.remove(client.file_header_index().unwrap());
    let mut errors: Vec<_> = checker
        .prepare_module_declarations(&client)
        .into_iter()
        .map(|e| e.to_string())
        .collect();
    if errors.is_empty() {
        for item in &client.items {
            if let Err(error) = checker.check_item(item) {
                errors.push(error.to_string());
                break;
            }
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| e.to_string()),
    );
    errors.extend(checker.diagnostic_sources().iter().map(|e| e.to_string()));
    errors
}
const PRELUDE: &str = "public mount realm.seed.Pivot;";
#[test]
fn imported_field_uses_the_declaring_files_prelude() {
    assert!(errors("module realm.provider;", "public", PRELUDE, "Int").is_empty());
}
#[test]
fn the_declaring_file_can_opt_out() {
    let result = errors(
        "@no_implicit_prelude module realm.provider;",
        "public",
        PRELUDE,
        "Int",
    );
    assert!(
        !result.is_empty(),
        "the caller cannot supply the omitted declaration scope"
    );
}
#[test]
fn private_prelude_mount_does_not_export_a_type() {
    assert!(
        !errors(
            "module realm.provider;",
            "public",
            "mount realm.seed.Pivot;",
            "Int"
        )
        .is_empty()
    );
}
#[test]
fn reexport_cannot_publish_a_private_declaration() {
    assert!(!errors("module realm.provider;", "private", PRELUDE, "Int").is_empty());
}
#[test]
fn an_explicit_missing_binding_cannot_fall_through_to_the_prelude() {
    assert!(
        !errors(
            "module realm.provider; mount realm.missing.Pivot;",
            "public",
            PRELUDE,
            "Int"
        )
        .is_empty()
    );
}
#[test]
fn local_declaration_precedes_the_prelude() {
    let source = "module realm.provider; type Pivot is { public code: Bool };";
    assert!(errors(source, "public", PRELUDE, "Bool").is_empty());
    assert!(!errors(source, "public", PRELUDE, "Int").is_empty());
}
